(Bracket top)
(CNDT 0.0.0-golden)
(POST grbl 0.1.0 UNVERIFIED)
(UNVERIFIED POST - PROVE OUT CAREFULLY BEFORE RUNNING)
(PREVIEW BUILD - NOT FOR PRODUCTION)
G21 G17 G90 G94
G54
(Face)
(T1 50 mm face mill)
M5
(Load tool 1, then resume)
M0
M3 S6000
M8
G0 X-30 Y0 Z5
Z0
G1 X70 F800
Y20
X-30
G0 Z5
(Circle)
(T2 6 mm flat end mill)
M5
(Load tool 2, then resume)
M0
M3 S12000
G0 X25 Y10 Z5
G1 Z-1.5 F200
G3 X15 Y10 I-5 J0 F600
G3 X25 Y10 I5 J0
G0 Z5
M1
(Drill)
(T3 5 mm drill)
M5
(Load tool 3, then resume)
M0
M3 S2500
(Check: clamps clear of holes)
G0 X5 Y5 Z5
G1 Z-8 F120
G4 P0.5
G0 Z5
X35
G1 Z-8
G4 P0.5
G0 Z5
M9
M5
M5
M30
