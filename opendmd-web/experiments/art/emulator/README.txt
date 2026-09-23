OpenDMD "native art" experiment: run an emulator core in the browser on a ROM the
owner opens locally, and grab title/attract frames as ImageData.

FILES
  ejs.html      EmulatorJS 4.2.3 page. The ROM comes from <input type=file> and is handed to
                EmulatorJS as a File object (EJS_gameUrl = file). Nothing is uploaded.
                Query options: core=gambatte|mgba|genesis_plus_gx  data=local|cdn
                  pdb=1 (force preserveDrawingBuffer)  hidden=1 (move emulator off-screen)
                  threads=1  suspendaudio=1 (simulate an autoplay-blocked AudioContext)
                  racfg=<retroarch.cfg lines, ';'-separated>, e.g. racfg=audio_sync%20%3D%20false
                window.grab.captureRaw()       raw core framebuffer (tex(Sub)Image2D tap), native res
                window.grab.captureRetroarch() RetroArch's own screenshot (PNG, native res)
                window.grab.captureCanvas()    drawImage of the WebGL canvas (scaled; blank without pdb=1)
  binjgb.html   binjgb (MIT) Game Boy core with no UI, audio or rAF: run N frames, read the
                RGBA framebuffer straight out of wasm memory.
  grab.py       Playwright driver (uv run). Serves this dir plus the vendored emulator files,
                picks the ROM through the file input, times boot, grabs frames to out/,
                tests fast-forward, pause and save states, and counts bytes per host.
  out/          sample PNG grabs (native resolution) plus *_report.json with the measurements.

NOTHING THIRD-PARTY IS IN THE REPO. Put the emulator files and ROMs somewhere else, e.g.:
  EJS_DATA   (default: <scratchpad>/vendor/ejs-data) = EmulatorJS "data/" directory:
     npm tarballs https://registry.npmjs.org/@emulatorjs/emulatorjs/-/emulatorjs-4.2.3.tgz
                  (package/data -> ejs-data/)
                  https://registry.npmjs.org/@emulatorjs/core-{gambatte,mgba,genesis_plus_gx}/-/core-<name>-4.2.3.tgz
                  (*.data -> ejs-data/cores/, reports/*.json -> ejs-data/cores/reports/)
     plus emulator.min.js and emulator.min.css from https://cdn.emulatorjs.org/stable/data/
     (the npm package ships only the unminified src/; the loader falls back to it if the min files are missing).
     You can also get everything in the release 7z: https://github.com/EmulatorJS/EmulatorJS/releases (4.2.3.7z, 290 MiB, all cores).
  BINJGB_DIR (default: <scratchpad>/vendor/binjgb) = binjgb.js + binjgb.wasm from
     https://github.com/binji/binjgb/tree/main/docs

TEST ROMS (legally redistributable homebrew; download, do NOT commit):
  Game Boy Color  uCity v1.3, GPLv3+ code, CC BY-SA 4.0 media
                  https://github.com/AntonioND/ucity/releases/download/v1.3/ucity.gbc
                  (ucity_compat.gbc = DMG-compatible build; source https://codeberg.org/SkyLyrac/ucity)
  Mega Drive      Miniplanets REMIX (REV04), zlib licence
                  https://github.com/sikthehedgehog/miniplanets ("Miniplanets REMIX Ver (REV04).bin" in repo root)
  Mega Drive      Lunch Break v1.0, GPL-3.0
                  https://github.com/GoodPraxis/lunch-break/releases/download/v1.0/lunchbreak.zip
  (Tobu Tobu Girl: MIT code + CC BY 4.0 assets, https://github.com/SimonLarsen/tobutobugirl, but the ROM
   is only on itch.io (https://tangramgames.itch.io/tobutobugirl), so it's not scriptable. Not used here.)

RUN
  cd opendmd-web/experiments/art/emulator
  uv run grab.py --rom /path/ucity.gbc --core gambatte --tag gb_gambatte
  uv run grab.py --rom /path/miniplanets.bin --core genesis_plus_gx --tag md_gpgx --start 120 --step 90
  uv run grab.py --rom /path/ucity.gbc --core gambatte --data cdn --n 3 --no-extras      # cores from cdn.emulatorjs.org
  uv run grab.py --rom /path/ucity.gbc --core gambatte --host 192.168.1.86 --no-extras    # plain-http LAN origin (not a secure context)
  uv run grab.py --rom /path/ucity.gbc --core gambatte --coi --threads --no-extras        # COOP/COEP + threaded core
  uv run grab.py --rom /path/ucity.gbc --core gambatte --suspendaudio --racfg "audio_sync = false"
  uv run grab.py --rom /path/ucity.gbc --engine binjgb --tag gb_binjgb
  Manual: run any static server that maps /ejs/ to EJS_DATA, open ejs.html, pick a core and a ROM.

RESULTS (Playwright Chromium, headless, SwiftShader WebGL, this machine, 2026-09-23)
  EmulatorJS, vendored locally:
    bytes    GB gambatte 1.71 MB (core .data 967 KB + emulator.min.js 426 KB + extract7z.js 280 KB + css/loader)
             MD genesis_plus_gx 1.95 MB (core .data 1.20 MB). The vendored subset for 3 cores (4 variants each) is 15 MB.
             A single variant per core (non-thread, legacy) is about 1 MB each.
    via CDN  GB 1.18 MB, MD 1.42 MB on the wire (the JS is brotli-compressed; the .data is already 7z).
    boot     file picked -> first core frame: 0.77-2.5 s local (GB about 0.85 s typical, MD 0.87-2.5 s);
             CDN 2.7-4.3 s. The spread comes from SwiftShader and the 7z unpack of the core.
    grabs    captureRaw: 160x144 (GB) / 320x224 (MD, 256x192 during the Sega/SMS-mode boot), RGB565.
             Copying the upload takes 0.04-0.2 ms. The call returns in 6-18 ms because it waits for the
             next emulated frame, then converts and PNG-encodes.
             captureRetroarch: native res PNG (video_gpu_screenshot=false), 55-100 ms per grab.
             captureCanvas: 640x480 scaled (non-integer 3.33x, nearest), about 5 ms. It is BLANK unless
             preserveDrawingBuffer is forced (pdb=1).
    speed    60 fps normal. Unlimited fast-forward (setFastForwardRatio(0)+toggleFastForward(1)):
             GB about 1400-3000 fps, MD about 500-770 fps.
             Pause (toggleMainLoop(0)) holds the frame count. Save states work: GB 182 KB, MD 1.0 MB,
             about 5 ms each way. Two loads of the same state give identical frames.
    audio    If the AudioContext stays suspended (autoplay block), emulation STALLS after the first frame,
             because RetroArch waits on audio. Adding "audio_sync = false" to retroarch.cfg
             (racfg=..., patched through EJS_GameManager.prototype.getRetroArchCfg) makes it run at 60 fps muted.
             Playwright Chromium never really blocks autoplay, so the stall was reproduced by forcing the
             context to suspend.
    origin   http://192.168.1.86 (not a secure context) works for non-threaded cores. COOP/COEP are ignored
             on an untrustworthy origin, so there is no SharedArrayBuffer. With threads=1 it never started.
             Threaded cores therefore need https or localhost. The non-threaded cores used here don't need them.
    network  Even when vendored, emulator.js fetches https://cdn.emulatorjs.org/stable/data/version.json
             (an update check). This is harmless, but it is a call to a third party.
  binjgb (GB only): 102 KB total download, 10 ms from file to first frame, 0.2 ms per grab (RGBA straight
    from wasm memory), about 2270 fps unthrottled, no audio or WebGL needed, deterministic save states (200 KB).
