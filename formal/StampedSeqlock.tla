---- MODULE StampedSeqlock ----
EXTENDS Naturals

CONSTANTS N, MaxIdx, RECHECK

Slots == 0..(N - 1)
SlotOf(i) == i % N

VARIABLES seq, stamp, payload, widx, wpc, rpc, ridx, s1, gotstamp, gotpayload, bad

vars == <<seq, stamp, payload, widx, wpc, rpc, ridx, s1, gotstamp, gotpayload, bad>>

Init ==
    /\ seq = [s \in Slots |-> 0]
    /\ stamp = [s \in Slots |-> 0]
    /\ payload = [s \in Slots |-> 0]
    /\ widx = 1
    /\ wpc = "begin"
    /\ rpc = "load1"
    /\ ridx = 1
    /\ s1 = 0
    /\ gotstamp = 0
    /\ gotpayload = 0
    /\ bad = FALSE

WBegin ==
    /\ wpc = "begin" /\ widx <= MaxIdx
    /\ seq' = [seq EXCEPT ![SlotOf(widx)] = 2 * widx - 1]
    /\ wpc' = "stamp"
    /\ UNCHANGED <<stamp, payload, widx, rpc, ridx, s1, gotstamp, gotpayload, bad>>

WStamp ==
    /\ wpc = "stamp"
    /\ stamp' = [stamp EXCEPT ![SlotOf(widx)] = widx]
    /\ wpc' = "payload"
    /\ UNCHANGED <<seq, payload, widx, rpc, ridx, s1, gotstamp, gotpayload, bad>>

WPayload ==
    /\ wpc = "payload"
    /\ payload' = [payload EXCEPT ![SlotOf(widx)] = widx]
    /\ wpc' = "publish"
    /\ UNCHANGED <<seq, stamp, widx, rpc, ridx, s1, gotstamp, gotpayload, bad>>

WPublish ==
    /\ wpc = "publish"
    /\ seq' = [seq EXCEPT ![SlotOf(widx)] = 2 * widx]
    /\ widx' = widx + 1
    /\ wpc' = "begin"
    /\ UNCHANGED <<stamp, payload, rpc, ridx, s1, gotstamp, gotpayload, bad>>

RLoad1 ==
    /\ rpc = "load1"
    /\ s1' = seq[SlotOf(ridx)]
    /\ rpc' = IF seq[SlotOf(ridx)] = 2 * ridx THEN "readstamp" ELSE "load1"
    /\ UNCHANGED <<seq, stamp, payload, widx, wpc, ridx, gotstamp, gotpayload, bad>>

RStamp ==
    /\ rpc = "readstamp"
    /\ gotstamp' = stamp[SlotOf(ridx)]
    /\ rpc' = "readpayload"
    /\ UNCHANGED <<seq, stamp, payload, widx, wpc, ridx, s1, gotpayload, bad>>

RPayload ==
    /\ rpc = "readpayload"
    /\ gotpayload' = payload[SlotOf(ridx)]
    /\ rpc' = "check"
    /\ UNCHANGED <<seq, stamp, payload, widx, wpc, ridx, s1, gotstamp, bad>>

RCheck ==
    /\ rpc = "check"
    /\ IF RECHECK = FALSE \/ seq[SlotOf(ridx)] = s1
       THEN /\ bad' = (bad \/ gotstamp # ridx \/ gotpayload # ridx)
            /\ ridx' = ridx + 1
            /\ rpc' = "load1"
       ELSE /\ UNCHANGED <<bad, ridx>>
            /\ rpc' = "load1"
    /\ UNCHANGED <<seq, stamp, payload, widx, wpc, s1, gotstamp, gotpayload>>

Next == WBegin \/ WStamp \/ WPayload \/ WPublish \/ RLoad1 \/ RStamp \/ RPayload \/ RCheck

Spec == Init /\ [][Next]_vars

NoMixedMessage == ~bad

====
