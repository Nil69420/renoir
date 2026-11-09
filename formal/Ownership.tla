---- MODULE Ownership ----
EXTENDS Naturals, FiniteSets

CONSTANTS N, R, MaxT, LEASE, FORCE, TIES

Producers == 1..N
Strength(p) == IF TIES THEN 1 ELSE p

VARIABLES reg, alive, fresh, belief, since, age, t

vars == <<reg, alive, fresh, belief, since, age, t>>

Eligible(rg, al, fr, p) == rg[p] /\ al[p] /\ (fr[p] \/ ~LEASE)

OwnerSet(rg, al, fr) ==
    LET E == {p \in Producers : Eligible(rg, al, fr, p)}
    IN {p \in E : \A q \in E : Strength(q) < Strength(p) \/ (Strength(q) = Strength(p) /\ q >= p)}

Owner(rg, al, fr) == IF OwnerSet(rg, al, fr) = {} THEN 0 ELSE CHOOSE p \in OwnerSet(rg, al, fr) : TRUE

Init ==
    /\ reg = [p \in Producers |-> FALSE]
    /\ alive = [p \in Producers |-> TRUE]
    /\ fresh = [p \in Producers |-> FALSE]
    /\ belief = [p \in Producers |-> FALSE]
    /\ since = [p \in Producers |-> R]
    /\ age = R
    /\ t = 0

Settle(rg, al, fr) ==
    /\ reg' = rg /\ alive' = al /\ fresh' = fr
    /\ age' = IF Owner(rg, al, fr) # Owner(reg, alive, fresh) THEN 0 ELSE age
    /\ UNCHANGED <<belief, since, t>>

Register(p) ==
    /\ ~reg[p] /\ alive[p]
    /\ Settle([reg EXCEPT ![p] = TRUE], alive, [fresh EXCEPT ![p] = TRUE])
    /\ UNCHANGED <<belief, since>>

Unregister(p) ==
    /\ reg[p]
    /\ Settle([reg EXCEPT ![p] = FALSE], alive, [fresh EXCEPT ![p] = FALSE])

Crash(p) ==
    /\ alive[p]
    /\ Settle(reg, [alive EXCEPT ![p] = FALSE], fresh)

Expire(p) ==
    /\ LEASE /\ reg[p] /\ alive[p] /\ fresh[p]
    /\ Settle(reg, alive, [fresh EXCEPT ![p] = FALSE])

Assert(p) ==
    /\ LEASE /\ reg[p] /\ alive[p] /\ ~fresh[p]
    /\ Settle(reg, alive, [fresh EXCEPT ![p] = TRUE])

Recheck(p) ==
    /\ reg[p] /\ alive[p] /\ since[p] >= R
    /\ belief' = [belief EXCEPT ![p] = (Owner(reg, alive, fresh) = p)]
    /\ since' = [since EXCEPT ![p] = 0]
    /\ UNCHANGED <<reg, alive, fresh, age, t>>

Tick ==
    /\ t < MaxT
    /\ t' = t + 1
    /\ since' = [p \in Producers |-> IF since[p] < R THEN since[p] + 1 ELSE R]
    /\ age' = IF age < R THEN age + 1 ELSE R
    /\ UNCHANGED <<reg, alive, fresh, belief>>

Next ==
    \/ \E p \in Producers : Register(p) \/ Unregister(p) \/ Crash(p) \/ Expire(p) \/ Assert(p) \/ Recheck(p)
    \/ Tick

Spec == Init /\ [][Next]_vars

CanWrite(p) == reg[p] /\ alive[p] /\ belief[p] /\ (FORCE = FALSE \/ since[p] < R)

NoDeadOrUnregisteredWriter == \A p \in Producers : CanWrite(p) => (alive[p] /\ reg[p])

BoundedOverlap ==
    \A p \in Producers :
        (CanWrite(p) /\ Owner(reg, alive, fresh) # p) => age < R

BoundedBlackout ==
    \A p \in Producers :
        (reg[p] /\ alive[p] /\ Owner(reg, alive, fresh) = p /\ ~CanWrite(p)) => (age < R \/ since[p] >= R)

UniqueTruthOwner == Cardinality(OwnerSet(reg, alive, fresh)) <= 1

====
