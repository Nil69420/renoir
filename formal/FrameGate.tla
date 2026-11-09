---- MODULE FrameGate ----
EXTENDS Naturals, FiniteSets

CONSTANTS N, MaxIdx, Consumers

VARIABLES write, pos, holding, dead, evicted, slotOwner, overwritten

vars == <<write, pos, holding, dead, evicted, slotOwner, overwritten>>

Slot(i) == i % N

Init ==
    /\ write = 0
    /\ pos = [c \in Consumers |-> 1]
    /\ holding = [c \in Consumers |-> 0]
    /\ dead = [c \in Consumers |-> FALSE]
    /\ evicted = [c \in Consumers |-> FALSE]
    /\ slotOwner = [s \in 0..(N - 1) |-> 0]
    /\ overwritten = FALSE

ActiveMin ==
    LET active == {c \in Consumers : ~evicted[c]} IN
    IF active = {} THEN write + 1 ELSE CHOOSE m \in {pos[c] : c \in active} : \A c \in active : m <= pos[c]

Produce ==
    /\ write < MaxIdx
    /\ write + 1 < ActiveMin + N
    /\ write' = write + 1
    /\ overwritten' = (overwritten \/ \E c \in Consumers : holding[c] # 0 /\ Slot(holding[c]) = Slot(write + 1) /\ holding[c] # write + 1 /\ ~dead[c])
    /\ slotOwner' = [slotOwner EXCEPT ![Slot(write + 1)] = write + 1]
    /\ UNCHANGED <<pos, holding, dead, evicted>>

View(c) ==
    /\ ~dead[c] /\ ~evicted[c] /\ holding[c] = 0
    /\ pos[c] <= write
    /\ holding' = [holding EXCEPT ![c] = pos[c]]
    /\ UNCHANGED <<write, pos, dead, evicted, slotOwner, overwritten>>

Release(c) ==
    /\ ~dead[c] /\ ~evicted[c] /\ holding[c] # 0
    /\ pos' = [pos EXCEPT ![c] = holding[c] + 1]
    /\ holding' = [holding EXCEPT ![c] = 0]
    /\ UNCHANGED <<write, dead, evicted, slotOwner, overwritten>>

Die(c) ==
    /\ ~dead[c]
    /\ dead' = [dead EXCEPT ![c] = TRUE]
    /\ UNCHANGED <<write, pos, holding, evicted, slotOwner, overwritten>>

Evict(c) ==
    /\ dead[c] /\ ~evicted[c]
    /\ evicted' = [evicted EXCEPT ![c] = TRUE]
    /\ UNCHANGED <<write, pos, holding, dead, slotOwner, overwritten>>

EvictAlive(c) ==
    /\ ~dead[c] /\ ~evicted[c]
    /\ evicted' = [evicted EXCEPT ![c] = TRUE]
    /\ UNCHANGED <<write, pos, holding, dead, slotOwner, overwritten>>

Next == \/ Produce \/ \E c \in Consumers : View(c) \/ Release(c) \/ Die(c) \/ Evict(c)

NextTimer == Next \/ \E c \in Consumers : EvictAlive(c)

SpecDeath == Init /\ [][Next]_vars
SpecTimer == Init /\ [][NextTimer]_vars

NoOverwriteWhileHeld == ~overwritten


====
