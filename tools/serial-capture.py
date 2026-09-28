# /// script
# requires-python = ">=3.11"
# dependencies = ["pyserial>=3.5"]
# ///
"""Capture ESP32 serial output WITHOUT resetting the board.

`cat /dev/ttyUSBx` and most terminal tools toggle DTR/RTS on open/close; on a
DevKit V1 that yanks EN and reboots (or holds) the chip. This opens the port
with DTR/RTS deasserted and leaves them alone on exit.
Usage: serial-capture.py PORT SECONDS [outfile] [--reset] [--baud=N]
--reset pulses EN via RTS (DTR low) so the boot log is captured, then runs.
--baud=N: the console's rate, 921600 by default (sdkconfig.defaults since
2026-09-28; firmware from before that logs at 115200). The ROM bootloader's
first lines after a reset are always 115200, so they read as garbage.
"""
import sys, time, serial
port, secs = sys.argv[1], float(sys.argv[2])
args = [a for a in sys.argv[3:] if not a.startswith("--")]
out = open(args[0], "ab") if args else None
s = serial.Serial()
baud = next((int(a.split("=", 1)[1]) for a in sys.argv if a.startswith("--baud=")), 921600)
s.port, s.baudrate, s.timeout = port, baud, 0.2
s.dtr = False; s.rts = False          # applied at open: no reset pulse
s.open()
s.dtr = False; s.rts = False
if "--reset" in sys.argv:
    s.rts = True; time.sleep(0.1); s.rts = False   # EN low then high: clean reboot
    s.reset_input_buffer()
t0 = time.time(); buf = b""
while time.time() - t0 < secs:
    chunk = s.read(4096)
    if not chunk: continue
    buf += chunk
    while b"\n" in buf:
        line, buf = buf.split(b"\n", 1)
        line = line.decode("utf-8", "replace").rstrip("\r")
        stamp = "%6.1f" % (time.time() - t0)
        print(stamp, line, flush=True)
        if out: out.write(f"{stamp} {line}\n".encode()); out.flush()
# deliberately no s.close() line-toggling: let the fd close with lines untouched
