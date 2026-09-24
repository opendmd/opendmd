//! The Studio's pipeline from the command line: the reference the web smoke
//! test compares the page's saved GIF with.
//!
//!     cargo run --release --example render -- IN.rgba W H OUT.rgb \
//!         [fit=pixels|half|fit|auto] [lift=0|1] [motion=still|pan|scroll|auto] \
//!         [x=N y=N] [step=40] [hold=1500] [key=0|1] [gain=100] [colours=0]
//!
//! `IN.rgba` is raw RGBA bytes (W x H). Writes every frame as raw RGB
//! (128x32x3 bytes each) to `OUT.rgb` and prints one JSON line: the scale
//! found, the fit, the stage size, the auto window, the window used, the
//! motion, the frame count, the delays and the palette size. Without `x`/`y`
//! the auto window for the motion is used, as the Studio does.

use opendmd_art_web::pipeline::{self, Colours, Fit, Image, Motion, Plan};
use std::collections::HashMap;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: render IN.rgba W H OUT.rgb [key=value ...]");
        return ExitCode::from(2);
    }
    let opts: HashMap<&str, &str> = args[4..].iter().filter_map(|a| a.split_once('=')).collect();
    let num = |k: &str, d: u32| opts.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let (w, h): (usize, usize) = (args[1].parse().unwrap(), args[2].parse().unwrap());
    let bytes = std::fs::read(&args[0]).expect("read input");
    let raw = Image::from_rgba(&bytes, w, h).expect("W x H x 4 bytes");
    let scale = pipeline::native_scale(&raw);
    let fit = match opts.get("fit").copied().unwrap_or("auto") {
        "auto" => pipeline::auto_fit(&raw, scale),
        f => Fit::parse(f),
    };
    let stage = pipeline::stage(&raw, scale, fit, num("lift", 0) == 1);
    let motion = match opts.get("motion").copied().unwrap_or("auto") {
        "auto" => pipeline::auto_motion(stage.w, stage.h),
        m => Motion::parse(m),
    };
    let auto = pipeline::auto_window(&stage, motion);
    let plan = Plan {
        motion,
        x: num("x", auto.0 as u32) as usize,
        y: num("y", auto.1 as u32) as usize,
        step_ms: num("step", 40),
        hold_ms: num("hold", 1500),
        key: num("key", 0) == 1,
        gain: num("gain", 100),
        colours: Colours::from_count(num("colours", 0)),
    };
    let anim = pipeline::render(&stage, &plan);
    let mut out = Vec::new();
    for i in 0..anim.frames.len() {
        out.extend(anim.rgb(i));
    }
    std::fs::write(&args[3], out).expect("write output");
    let (_, wins) = pipeline::windows(stage.w, stage.h, motion, plan.x, plan.y);
    println!(
        "{{\"scale\":{},\"fit\":\"{}\",\"stage\":[{},{}],\"auto_window\":[{},{}],\"window\":[{},{}],\"motion\":\"{}\",\"frames\":{},\"delays\":{:?},\"palette\":{}}}",
        scale.k,
        fit.name(),
        stage.w,
        stage.h,
        auto.0,
        auto.1,
        wins[0].0,
        wins[0].1,
        motion.name(),
        anim.frames.len(),
        anim.delays_ms,
        anim.palette.len()
    );
    ExitCode::SUCCESS
}
