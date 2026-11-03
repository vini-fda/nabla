---------------------------- MODULE NablaNet ----------------------------
EXTENDS Naturals, Sequences

CONSTANTS Ships, Cmds, Neutral, None, Window, Drop, MaxTick
ASSUME Neutral \notin Cmds /\ None \notin Cmds \cup {Neutral}

Stamps == 0 .. (MaxTick + 1)

VARIABLES T, x, last, miss, buf,   \* the server
          k, cmd, baseT, baseX,    \* the clients
          up, down                 \* datagrams in flight

vars == <<T, x, last, miss, buf, k, cmd, baseT, baseX, up, down>>

Init ==
  /\ T = 0
  /\ x = [i \in Ships |-> <<>>]
  /\ last = [i \in Ships |-> Neutral]
  /\ miss = [i \in Ships |-> 0]
  /\ buf = [i \in Ships |-> [s \in Stamps |-> None]]
  /\ k = [i \in Ships |-> 0]
  /\ cmd = [i \in Ships |-> [s \in Stamps |-> None]]
  /\ baseT = [i \in Ships |-> 0]
  /\ baseX = [i \in Ships |-> <<>>]
  /\ up = {}
  /\ down = {}

Issue(i, c) ==
  /\ k[i] <= MaxTick
  /\ cmd' = [cmd EXCEPT ![i][k[i]] = c]
  /\ up' = up \cup {[ship |-> i, tick |-> k[i], cmd |-> c]}
  /\ k' = [k EXCEPT ![i] = @ + 1]
  /\ UNCHANGED <<T, x, last, miss, buf, baseT, baseX, down>>

\* Discard: a stamp below T, or at or above T + Window, is not deliverable.
Deliver(m) ==
  /\ m \in up
  /\ T <= m.tick /\ m.tick < T + Window
  /\ buf[m.ship][m.tick] = None
  /\ buf' = [buf EXCEPT ![m.ship][m.tick] = m.cmd]
  /\ UNCHANGED <<T, x, last, miss, k, cmd, baseT, baseX, up, down>>

Chosen(i) ==
  IF buf[i][T] # None THEN buf[i][T]
  ELSE IF miss[i] + 1 < Drop THEN last[i]
  ELSE Neutral

Tick ==
  /\ T < MaxTick
  /\ x' = [i \in Ships |-> Append(x[i], Chosen(i))]
  /\ last' = [i \in Ships |-> Chosen(i)]
  /\ miss' = [i \in Ships |-> IF buf[i][T] # None THEN 0 ELSE miss[i] + 1]
  /\ buf' = [i \in Ships |-> [buf[i] EXCEPT ![T] = None]]
  /\ T' = T + 1
  /\ down' = down \cup {[ship |-> i, tick |-> T + 1,
                         x |-> Append(x[i], Chosen(i))] : i \in Ships}
  /\ UNCHANGED <<k, cmd, baseT, baseX, up>>

\* The guard keeps the base monotone. The update of k is the Advance rule.
Absorb(m) ==
  /\ m \in down
  /\ m.tick > baseT[m.ship]
  /\ baseT' = [baseT EXCEPT ![m.ship] = m.tick]
  /\ baseX' = [baseX EXCEPT ![m.ship] = m.x]
  /\ k' = [k EXCEPT ![m.ship] = IF @ < m.tick THEN m.tick ELSE @]
  /\ UNCHANGED <<T, x, last, miss, buf, cmd, up, down>>

LoseUp(m) ==
  /\ m \in up
  /\ up' = up \ {m}
  /\ UNCHANGED <<T, x, last, miss, buf, k, cmd, baseT, baseX, down>>

LoseDown(m) ==
  /\ m \in down
  /\ down' = down \ {m}
  /\ UNCHANGED <<T, x, last, miss, buf, k, cmd, baseT, baseX, up>>

Next ==
  \/ \E i \in Ships, c \in Cmds : Issue(i, c)
  \/ \E m \in up : Deliver(m) \/ LoseUp(m)
  \/ Tick
  \/ \E m \in down : Absorb(m) \/ LoseDown(m)

Spec == Init /\ [][Next]_vars /\ WF_vars(Tick)
                              /\ SF_vars(\E m \in down : Absorb(m))

Prefix(s, n) == SubSeq(s, 1, n)

\* The prediction: a stamp the client skipped replays as the command before it.
RECURSIVE Pred(_, _)
Pred(i, n) ==
  IF n <= baseT[i] THEN baseX[i]
  ELSE LET p == Pred(i, n - 1)
           held == IF Len(p) = 0 THEN Neutral ELSE p[Len(p)]
       IN  Append(p, IF cmd[i][n - 1] # None THEN cmd[i][n - 1] ELSE held)

TickIntegrity == \A i \in Ships : Len(x[i]) = T

BufferBound ==
  \A i \in Ships, s \in Stamps : buf[i][s] # None => T <= s /\ s < T + Window

SnapshotTruth == \A i \in Ships : baseX[i] = Prefix(x[i], baseT[i])

BufferFidelity ==
  \A i \in Ships, s \in Stamps : buf[i][s] # None => buf[i][s] = cmd[i][s]

Provenance ==
  \A i \in Ships : \A n \in 1 .. Len(x[i]) :
    LET c == x[i][n]
        prev == IF n = 1 THEN Neutral ELSE x[i][n - 1]
    IN  c = cmd[i][n - 1] \/ c = prev \/ c = Neutral

Lead == \A i \in Ships : baseT[i] <= k[i]

\* Where the replay agrees with the log since the base, it equals the log.
ExactPrediction ==
  \A i \in Ships : \A n \in baseT[i] .. (IF k[i] < T THEN k[i] ELSE T) :
    (\A t \in (baseT[i] + 1) .. n : Pred(i, n)[t] = x[i][t])
      => Pred(i, n) = Prefix(x[i], n)

HistoryStable ==
  [][\A i \in Ships : Prefix(x'[i], Len(x[i])) = x[i]]_vars

BaseMonotone == [][\A i \in Ships : baseT'[i] >= baseT[i]]_vars
=========================================================================
