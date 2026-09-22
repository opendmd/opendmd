# DMDos Board v3 — measured pinout (ESP32 DevKit V1, 30-pin)

Measured 2026-09-22 by continuity walk (MicroPython pin-walker on a spare DevKit +
spare carrier; GND jumper touched to each header hole). Logs: pinwalk.run1.log /
pinwalk.run2.log. Orientation: board back facing you, microSD slot at top,
"DMD IN" header on the left. Outer column = even HUB75 pins, inner (notched)
column = odd pins, row 1 at the top.

## HUB75 "DMD IN" (2x8 IDC)

| Row | Odd pin (inner) | GPIO | Even pin (outer) | GPIO |
|----:|-----------------|-----:|------------------|-----:|
| 1 | 1  R1  | 25 | 2  G1  | 26 |
| 2 | 3  B1  | 27 | 4  GND | –  |
| 3 | 5  R2  | 14 | 6  G2  | 12 |
| 4 | 7  B2  | 13 | 8  E/GND | – (silent; E unused at 1/16 scan) |
| 5 | 9  A   | 33 | 10 B   | 32 |
| 6 | 11 C   | 22 | 12 D   | 17 |
| 7 | 13 CLK | 16 | 14 LAT | 4  |
| 8 | 15 OE  | 15 | 16 GND | –  |

ESP32-HUB75-MatrixPanel-DMA config:
R1=25 G1=26 B1=27 R2=14 G2=12 B2=13 A=33 B=32 C=22 D=17 E=-1 LAT=4 OE=15 CLK=16

Sanity check: this is the library's ESP32 default map with only A/B/C moved
(23→33, 19→32, 5→22) — precisely the three defaults that collide with the SD SPI bus.

## microSD (SPI, on the 6-pin socket module — NOT SDMMC)

VSPI defaults: SCK=18, MISO=19, MOSI=23, CS=5. Module also has 3V3 and GND
(3V3 is the last hole on the back-side row; shorting it to GND resets the board).
Roles CONFIRMED 2026-09-22: SDCard init succeeds and a raw CMD0 probe gets R1=0x01
only with sck=18 miso=19 mosi=23 cs=5 (all 23 other role permutations silent).
Tested against the 512GB exFAT card (read-only; exFAT won't mount in MicroPython's
FAT VFS, which is expected — the FAT32 DMDoS card will).

## Power

DMDPower screw terminal: RED +5V → DevKit VIN, BLACK → GND. The ESP32 runs from
the panel PSU.

## Unused

GPIO 2 (onboard LED), 21, 34, 35, 36, 39, UART0 (1/3).
