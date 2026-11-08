---- MODULE LeanMP ----
EXTENDS Naturals, FiniteSets

CONSTANTS N, P, MaxIdx, MODE

Producers == 1..P
Slots == 0..(N - 1)
CL == 99

VARIABLES claim, seq, data, pc, tmp, inflight, dead, rnext, lost, bad

vars == <<claim, seq, data, pc, tmp, inflight, dead, rnext, lost, bad>>

SlotOf(i) == i % N
Prev(i) == IF i > N THEN 2 * (i - N) ELSE 0

Init ==
    /\ claim = 0
    /\ seq = [s \in Slots |-> 0]
    /\ data = [s \in Slots |-> 0]
    /\ pc = [p \in Producers |-> "idle"]
    /\ tmp = [p \in Producers |-> 0]
    /\ inflight = [p \in Producers |-> 0]
    /\ dead = [p \in Producers |-> FALSE]
    /\ rnext = 1
    /\ lost = {}
    /\ bad = FALSE

AliveCovering(i) ==
    \E q \in Producers : ~dead[q] /\ (inflight[q] = CL \/ inflight[q] = i)

Begin(p) ==
    /\ pc[p] = "idle" /\ ~dead[p] /\ claim < MaxIdx
    /\ inflight' = [inflight EXCEPT ![p] = CL]
    /\ pc' = [pc EXCEPT ![p] = "claim"]
    /\ UNCHANGED <<claim, seq, data, tmp, dead, rnext, lost, bad>>

Claim(p) ==
    /\ pc[p] = "claim" /\ ~dead[p]
    /\ claim < MaxIdx
    /\ claim + 1 < rnext + N
    /\ claim' = claim + 1
    /\ tmp' = [tmp EXCEPT ![p] = claim + 1]
    /\ pc' = [pc EXCEPT ![p] = "mark"]
    /\ UNCHANGED <<seq, data, inflight, dead, rnext, lost, bad>>

Mark(p) ==
    /\ pc[p] = "mark" /\ ~dead[p]
    /\ inflight' = [inflight EXCEPT ![p] = tmp[p]]
    /\ pc' = [pc EXCEPT ![p] = "acquire"]
    /\ UNCHANGED <<claim, seq, data, tmp, dead, rnext, lost, bad>>

Acquire(p) ==
    LET i == tmp[p] IN
    /\ pc[p] = "acquire" /\ ~dead[p]
    /\ seq[SlotOf(i)] = Prev(i)
    /\ seq' = [seq EXCEPT ![SlotOf(i)] = 2 * i - 1]
    /\ pc' = [pc EXCEPT ![p] = "write"]
    /\ UNCHANGED <<claim, data, tmp, inflight, dead, rnext, lost, bad>>

TakeOver(p) ==
    LET i == tmp[p]
        cur == seq[SlotOf(i)]
        stuck == IF cur % 2 = 1 THEN (cur + 1) \div 2 ELSE i - N
    IN
    /\ pc[p] = "acquire" /\ ~dead[p]
    /\ cur # Prev(i) /\ cur < 2 * i - 1
    /\ (MODE = "OLD" \/ ~AliveCovering(stuck))
    /\ seq' = [seq EXCEPT ![SlotOf(i)] = 2 * i - 1]
    /\ pc' = [pc EXCEPT ![p] = "write"]
    /\ UNCHANGED <<claim, data, tmp, inflight, dead, rnext, lost, bad>>

Write(p) ==
    /\ pc[p] = "write" /\ ~dead[p]
    /\ data' = [data EXCEPT ![SlotOf(tmp[p])] = tmp[p]]
    /\ pc' = [pc EXCEPT ![p] = "commit"]
    /\ UNCHANGED <<claim, seq, tmp, inflight, dead, rnext, lost, bad>>

Commit(p) ==
    LET i == tmp[p] IN
    /\ pc[p] = "commit" /\ ~dead[p]
    /\ seq' = IF seq[SlotOf(i)] = 2 * i - 1
              THEN [seq EXCEPT ![SlotOf(i)] = 2 * i]
              ELSE seq
    /\ inflight' = [inflight EXCEPT ![p] = 0]
    /\ pc' = [pc EXCEPT ![p] = "idle"]
    /\ UNCHANGED <<claim, data, tmp, dead, rnext, lost, bad>>

Crash(p) ==
    /\ ~dead[p] /\ pc[p] # "idle"
    /\ dead' = [dead EXCEPT ![p] = TRUE]
    /\ UNCHANGED <<claim, seq, data, pc, tmp, inflight, rnext, lost, bad>>

Read ==
    LET s == SlotOf(rnext) IN
    /\ seq[s] = 2 * rnext
    /\ bad' = (bad \/ data[s] # rnext)
    /\ rnext' = rnext + 1
    /\ UNCHANGED <<claim, seq, data, pc, tmp, inflight, dead, lost>>

Skip ==
    /\ rnext <= claim
    /\ seq[SlotOf(rnext)] # 2 * rnext
    /\ (MODE = "OLD" \/ ~AliveCovering(rnext))
    /\ lost' = lost \cup {rnext}
    /\ rnext' = rnext + 1
    /\ UNCHANGED <<claim, seq, data, pc, tmp, inflight, dead, bad>>

Next ==
    \/ \E p \in Producers :
        Begin(p) \/ Claim(p) \/ Mark(p) \/ Acquire(p) \/ TakeOver(p)
        \/ Write(p) \/ Commit(p) \/ Crash(p)
    \/ Read \/ Skip

Spec == Init /\ [][Next]_vars

NoTornRead == ~bad

NoLossOfLiveProducer ==
    \A i \in lost : \E q \in Producers : tmp[q] = i /\ dead[q]

====
