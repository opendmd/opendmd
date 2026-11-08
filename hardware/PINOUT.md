# DMDoS board v3 pinout (ESP32 DevKit V1, 30-pin)

Measured by continuity test on the original DMDoS carrier board. Orientation: board back facing you, microSD slot at top,
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

This is the library's ESP32 default map with only A, B and C moved
(23→33, 19→32, 5→22), the three defaults that collide with the SD card's SPI bus.

## microSD (SPI on the 6-pin socket module, not SDMMC)

The card is on VSPI's default pins: SCK=18, MISO=19, MOSI=23, CS=5. They are
VSPI's IO_MUX pins, so the firmware drives them from SPI3 (VSPI); SPI2 (HSPI)
would route them through the GPIO matrix, which caps reads at 26.7 MHz. The
clock is 20 MHz by default, and 40 MHz only with `card_speed` set to fast
(`SD_SPEED_KHZ` in config.rs is the 40 MHz clock). The module also has 3V3
and GND. 3V3 is the last hole on the back-side row, and shorting it to GND
resets the board. The card answers only with this assignment of the four
pins; every other permutation is silent.

## Power

DMDPower screw terminal: RED +5V → DevKit VIN, BLACK → GND. The ESP32 runs from
the panel PSU.

## Unused

GPIO 2 (onboard LED), 21, 34, 35, 36, 39, UART0 (1/3).
