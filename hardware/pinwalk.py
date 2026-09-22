# MicroPython pin-walker for a bare ESP32 DevKit V1 on the DMDos Board v3.
# Every output-capable GPIO on the 30-pin header is set INPUT + PULL_UP.
# Touch a jumper from a DevKit GND pin to a header hole; the GPIO wired to
# that hole reads LOW and is reported once per touch.
from machine import Pin
import time
CANDIDATES = [32,33,25,26,27,14,12,13, 23,22,21,19,18,5,17,16,4,2,15]
pins = {g: Pin(g, Pin.IN, Pin.PULL_UP) for g in CANDIDATES}
time.sleep_ms(50)
base = {g: p.value() for g, p in pins.items()}
low0 = [g for g, v in base.items() if v == 0]
print("PINWALK READY. candidates:", CANDIDATES)
print("stuck low at start (external pulldown or shorted):", low0 or "none")
prev = dict(base)
n = 0
t0 = time.ticks_ms()
while True:
    for g, p in pins.items():
        v = p.value()
        if v != prev[g]:
            time.sleep_ms(15)              # debounce
            if p.value() != v: continue
            prev[g] = v
            if v == 0:
                n += 1
                print("TOUCH %2d: GPIO %d LOW  t=%.1fs" % (n, g, time.ticks_diff(time.ticks_ms(), t0)/1000))
            else:
                print("         GPIO %d released" % g)
    time.sleep_ms(5)
