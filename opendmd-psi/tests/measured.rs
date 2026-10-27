// Measured on the spare by psi-probe (2026-10-26, 1.0.0-beta.10-anylook-a): VM p50 µs a frame.
pub const STEPS: &[(&str, &str, u32)] = &[
    ("base gray(x)", "gray(x)", 678),
    ("add", "gray(x + t)", 897),
    ("mul", "gray(x * t)", 1041),
    ("div (+add)", "gray(x / (x + t))", 3697),
    ("mod 0.3", "gray(x % 0.3)", 1152),
    ("sin (+add)", "gray(sin(x + t))", 1346),
    ("sqrt (+add)", "gray(sqrt(x + t))", 3095),
    ("len (+add)", "gray(len(x, x + t))", 3526),
    ("atan2 (+sub)", "gray(atan2(y - 0.5, x - t))", 3032),
    ("smoothstep (+add)", "gray(smoothstep(0.2, 0.8, x + t))", 1782),
    ("noise (+mul)", "gray(noise(x * 8, y, t))", 4540),
    ("select (+lt)", "gray(x < t ? x : 0.5)", 1064),
    ("digit (+2 mask)", "gray(digit(px % 8, px % 4, y * 6))", 2066),
    ("bit", "gray(bit(px, t))", 915),
    ("pick (+mul)", "gray(pick(x * 4, t, 0.5, 0.75, 1))", 1452),
    ("pick 8 (+mul)", "gray(pick(x * 8, t, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7))", 1452),
    ("chain 4 (+mul)", "k = x * 4\ngray(k < 0 ? 0 : k < 1 ? t : k < 2 ? 0.5 : k < 3 ? 0.75 : k < 4 ? 1 : 0)", 3015),
    ("big clock", "loop 60\n-- Big clock: the time in four big digits, and a seconds bar along the bottom\nframe {\n  h1 = floor(hour/10); h2 = hour % 10       -- the hours' two digits\n  m1 = floor(minute/10); m2 = minute % 10   -- the minutes' two digits\n  end = (second + 1)*128/60                 -- where the seconds bar ends\n}\n-- a digit: the bold font's 3 x 5 pixels, each 8 px wide, 6 tall (24 x 30)\nq = px >= 64 ? px - 8 : px     -- the minutes sit 8 px further right\nk = floor(q/32)                -- which digit: one every 32 px (0 1, then 2 3)\nn = pick(k, h1, h2, m1, m2)    -- and its value\ncol = floor(q % 32 / 8)        -- the font's column, 0 to 2 (then the gap)\nrow = floor(ty/6)              -- its row from the top, 0 to 4 (5: below)\nbar = py < 1                   -- the bottom row is the seconds bar\nlit = bar ? px < end : digit(n, col, row, bold)\n-- a rainbow drifting through the digits, tilted by row; the bar is white\nhue = py/64 + hour/24 + t/10\nhsv(x + hue, bar ? 0 : 1, lit)", 6121),
    ("big clock (beta.10)", "loop 60\n-- Big clock: the time in four big digits, and a seconds bar along the bottom\nframe {\n  h1 = floor(hour/10); h2 = hour % 10       -- the hours' two digits\n  m1 = floor(minute/10); m2 = minute % 10   -- the minutes' two digits\n  end = (second + 1)*128/60                 -- where the seconds bar ends\n}\n-- a digit: the bold font's 3 x 5 pixels, each 8 px wide, 6 tall (24 x 30)\nq = px >= 64 ? px - 8 : px     -- the minutes sit 8 px further right\nk = floor(q/32)                -- which digit: one every 32 px (0 1, then 2 3)\nn = k < 1 ? h1 : k < 2 ? h2 : k < 3 ? m1 : m2   -- and its value\ncol = floor(q % 32 / 8)        -- the font's column, 0 to 2 (then the gap)\nrow = floor((31 - py)/6)       -- its row from the top, 0 to 4 (5: below)\nbar = py < 1                   -- the bottom row is the seconds bar\nlit = bar ? px < end : digit(n, col, row, bold)\n-- a rainbow drifting through the digits, tilted by row; the bar is white\nhue = py/64 + hour/24 + t/10\nhsv(x + hue, bar ? 0 : 1, lit)", 7275),
    ("out hsv", "hsv(x, 1, 1)", 1511),
    ("out pal", "pal(fire, x)", 610),
    ("ur", "gray(ur)", 962),
    ("ul", "gray(ul)", 1256),
    ("shows through", "gray(x, 0.5)", 2135),
    ("+layer over 0.5", "layer { gray(x) }\nlayer { gray(x + t, 0.5) }", 2949),
    ("+layer add", "layer { gray(x) }\nlayer add { gray(x + t) }", 2128),
    ("+layer multiply", "layer { gray(x) }\nlayer multiply { gray(x + t) }", 2349),
    ("+layer screen", "layer { gray(x) }\nlayer screen { gray(x + t) }", 2575),
    ("+layer max", "layer { gray(x) }\nlayer max { gray(x + t) }", 2077),
    ("look scanlines", "layer multiply { gray(py % 2 ? 1 : 0.55) }", 1098),
    ("look 4 layers", "layer multiply { gray(y) }\nlayer add { gray(x) }\nlayer screen { gray(x + t) }\nlayer max { gray(py / 32) }", 4957),
    ("+layer add 0.4", "layer { gray(x) }\nlayer add { gray(x + t, 0.4) }", 3707),
    ("+layer multiply 0.4", "layer { gray(x) }\nlayer multiply { gray(x + t, 0.4) }", 3929),
    ("+layer screen 0.4", "layer { gray(x) }\nlayer screen { gray(x + t, 0.4) }", 3789),
    ("+layer max 0.4", "layer { gray(x) }\nlayer max { gray(x + t, 0.4) }", 3174),
    ("+layer min 0.4", "layer { gray(x) }\nlayer min { gray(x + t, 0.4) }", 3191),
    ("+layer screen row", "layer { gray(x) }\nlayer screen { gray(y + t) }", 1918),
    ("+layer screen hsv", "layer { gray(x) }\nlayer screen { hsv(x + t, 1, 1) }", 3407),
    ("screen 0.4 on picture", "layer screen { gray(x + t, 0.4) }", 3193),
    ("hsv 0.3 on picture", "hsv(x + t, 1, 1, 0.3)", 3186),
    ("screen hsv 0.4 on picture", "layer screen { hsv(x + t, 1, 1, 0.4) }", 4405),
];
pub const LOOKS: &[(&str, &str, &str, u8, u32)] = &[
    ("Plasma", "loop 8\n-- three waves added together\nwave = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(x*1.5 + y + t/8)\nhsv(wave/6 + t/8, 0.9, 1)", "screen", 40, 7474),
    ("Plasma", "loop 8\n-- three waves added together\nwave = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(x*1.5 + y + t/8)\nhsv(wave/6 + t/8, 0.9, 1)", "over", 30, 6257),
    ("Plasma", "loop 8\n-- three waves added together\nwave = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(x*1.5 + y + t/8)\nhsv(wave/6 + t/8, 0.9, 1)", "add", 40, 7015),
    ("Plasma", "loop 8\n-- three waves added together\nwave = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(x*1.5 + y + t/8)\nhsv(wave/6 + t/8, 0.9, 1)", "multiply", 100, 5653),
    ("Plasma", "loop 8\n-- three waves added together\nwave = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(x*1.5 + y + t/8)\nhsv(wave/6 + t/8, 0.9, 1)", "max", 50, 6858),
    ("Fire", "loop 4\n-- curl bends the flames, more and more as they rise\ncurl = sin(y*1.5 - t)*0.25 + sin(y*3 - t*2 + x*4)*0.15\nflames = sin(x*6 + curl)*0.5 + sin(x*11 - curl)*0.5\nheat = (1 - y)*(1.6 + flames*0.7) - 0.5\npal(fire, clamp(heat, 0, 0.95))", "screen", 40, 8889),
    ("Fire", "loop 4\n-- curl bends the flames, more and more as they rise\ncurl = sin(y*1.5 - t)*0.25 + sin(y*3 - t*2 + x*4)*0.15\nflames = sin(x*6 + curl)*0.5 + sin(x*11 - curl)*0.5\nheat = (1 - y)*(1.6 + flames*0.7) - 0.5\npal(fire, clamp(heat, 0, 0.95))", "over", 30, 8054),
    ("Rings", "loop 4\n-- r: the distance from the middle\npal(rainbow, frac(r*1.5 - t/2))", "screen", 40, 6614),
    ("Stripes", "loop 4\n-- a: stripe width\nsize = 8 + a*24\nband = sin((px + py)/size - t/2)\ns = clamp(band*1.5 + 0.5, 0, 1)\nrgb(1, s, s)", "screen", 40, 5525),
    ("Copper bars", "loop 4\n-- each bar is brightest in its middle, in steps like the old chips\nred = floor(max(0, 1 - abs(cy - 0.35*sin(t/4))*5)*6)/6\ngold = floor(max(0, 1 - abs(cy - 0.35*sin(t/4 - 0.12))*5)*6)/6\nblue = floor(max(0, 1 - abs(cy - 0.35*sin(t/4 - 0.24))*5)*6)/6\nrgb(max(red, gold), max(gold*0.8, blue*0.6), blue)", "screen", 40, 2854),
    ("Old photo", "-- a: how much (0: the GIF as it is)\nmix(rgb(ur, ug, ub), rgb(ul*1.1 + 0.08, ul*0.92 + 0.04, ul*0.65), a)", "screen", 50, 9122),
    ("Old TV", "loop 4\ndx = (x - 0.5)*1.6; dy = (y - 0.5)*1.2\nlines = py % 2 ? 1 : 0.6\nroll = 0.85 + 0.15*max(0, tri(y/2 + t/4))\ncorners = clamp(1.5 - (dx*dx + dy*dy)*1.6, 0, 1)\nlayer multiply { gray(lines * roll * corners) }", "over", 100, 3958),
];
// The draw stage (psi-probe 2026-10-27, 1.0.0-beta.11-drawprobe-a): VM p50
// µs a frame, as STEPS.
pub const DRAW: &[(&str, &str, u32)] = &[
    ("draw base (1 dot off)", "draw { dot(-1, -1, t) }\ngray(ink)", 931),
    ("draw loop step: 1024 dots off", "draw { for k < 1024 { dot(-1, -1, t) } }\ngray(ink)", 1694),
    ("draw loop step: +add", "draw { for k < 1024 { dot(-1, -1, k + t) } }\ngray(ink)", 2124),
    ("draw loop step: +hash", "draw { for k < 1024 { dot(-1, -1, hash(k, t)) } }\ngray(ink)", 2235),
    ("draw dot on panel x1024", "draw { for k < 1024 { dot(k % 128, 5 + t) } }\ngray(ink)", 2295),
    ("draw line 12 x256", "draw { for k < 256 { line(10, 5, 21, 5 + t) } }\ngray(ink)", 2257),
    ("draw line 100 x64", "draw { for k < 64 { line(10, 5, 109, 5 + t) } }\ngray(ink)", 3041),
];
// Formulas that draw played as effects (the real path, `Fx::frame`, slider
// a = 0.5): p50 µs a frame, compared with the estimate as it is (it includes
// the real paths' ×1.15). The sources are the examples as measured
// (1.0.0-beta.11-drawprobe-a), before the eye test's changes of 2026-10-27
// (formulas.json: longer streaks, heads off the panel skipped, squared
// brightness, `fps 100`; a bigger cube): kept, because a measurement is of
// its source. The new examples estimate 6.0 ms (Starfield) and 2.9 ms
// (cube); psi-probe times them at its next run.
pub const EFFECTS: &[(&str, &str, u32)] = &[
    ("Starfield", STARFIELD, 4410),
    ("Wireframe cube", CUBE, 2678),
];
pub const STARFIELD: &str = "loop 2.4\n-- Starfield: 192 stars flying at you, a pass through the field a loop,\n-- each a streak from where it was a moment ago. a: streak length\nframe { back = 8 + a*240 }\ndraw {\n  for k < 192 {\n    sx = (hash(k, 7) - 0.5)*4096; sy = (hash(k, 3) - 0.5)*1024  -- where it is, across and up\n    z = frac(hash(k, 11) - t/2.4)*4096        -- how far: 0 near, 4096 far\n    s = 2/(z/16 + 4); st = 2/(min(z + back, 4095)/16 + 4)  -- perspective, now and before\n    lit = 1 - z/4096                          -- fading in from the dark\n    line(64 + sx*st, 16 + sy*st, 64 + sx*s, 16 + sy*s, lit*0.2, lit)\n  }\n}\npal(ice, ink)";
pub const CUBE: &str = "loop 4.8\n-- Wireframe cube: turning about two axes and swinging across; the near\n-- edges brighter. a: size. The colour is the last line's #40c0ff\ntable V = -1,-1,-1, 1,-1,-1, 1,1,-1, -1,1,-1, -1,-1,1, 1,-1,1, 1,1,1, -1,1,1\ntable E = 0,1, 1,2, 2,3, 3,0, 4,5, 5,6, 6,7, 7,4, 0,4, 1,5, 2,6, 3,7\nframe {\n  sa = sin(t/4.8); ca = cos(t/4.8); sb = sin(t/4.8 + 1/6); cb = cos(t/4.8 + 1/6)\n  size = 20 + a*28; mid = 64 + sa*40\n}\ndraw {\n  for e < 12 {\n    m = E[e*2]*3; n = E[e*2 + 1]*3            -- the edge's two corners\n    x0 = V[m]*size; y0 = V[m + 1]*size; z0 = V[m + 2]*size\n    x1 = V[n]*size; y1 = V[n + 1]*size; z1 = V[n + 2]*size\n    xa = x0*ca + z0*sa; za = z0*ca - x0*sa; ya = y0*cb - za*sb; zb = y0*sb + za*cb  -- turned\n    xc = x1*ca + z1*sa; zc = z1*ca - x1*sa; yc = y1*cb - zc*sb; zd = y1*sb + zc*cb\n    line(mid + xa*36/(zb + 128), 16 + ya*36/(zb + 128), mid + xc*36/(zd + 128), 16 + yc*36/(zd + 128),\n         clamp(0.6 - zb/90, 0.16, 1), clamp(0.6 - zd/90, 0.16, 1))\n  }\n}\n#40c0ff * ink";
