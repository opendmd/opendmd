"""M4, M5 and M6's shared part: a job of N handshakes at an interval, sampled."""
import s0lib


def run_job(run, args, phase: str, resume: bool, n: int, interval_s: float, timeout_s: float):
    st = run.sample(f"{phase}:before")
    before = st.get("connect") or {}
    run.panel.post("handshakes", n=n, interval_ms=int(interval_s * 1000), resume=int(resume))
    print(f"  asked {n} {'resumed' if resume else 'full'} handshakes {interval_s:g} s apart", flush=True)
    seen = {"k": 0}

    def progress(s):
        d = s0lib.get(s, "connect.job.done") or 0
        if d // 10 != seen["k"]:
            seen["k"] = d // 10
            c = s.get("connect") or {}
            h = c.get("resumed" if resume else "full") or {}
            print(f"    {d}/{n}: last {h.get('last')} ms, max {h.get('max')}, heap hs_low {s0lib.get(c, 'heap.hs_low')}, floor {s0lib.get(s, 'heap.floor')}", flush=True)

    def done(s):
        c = s.get("connect") or {}
        return s0lib.get(c, "job.kind") == "none" and (s0lib.get(c, "job.done") or 0) >= n

    st = run.sample_for(timeout_s, phase, until=done, every=progress)
    after = (st or {}).get("connect") or {}
    return before, after


def handshake_gates(run, phase, before, after, resume: bool, n: int, max_ms: int):
    h = after.get("resumed" if resume else "full") or {}
    hb = before.get("resumed" if resume else "full") or {}
    got = (h.get("n") or 0) - (hb.get("n") or 0)
    run.gate(f"{phase}: {'resumed' if resume else 'full'} handshakes done", got >= (n - 1 if resume else n),
             f"{got} of {n} (job failed {s0lib.get(after, 'job.failed')})", f">= {n - 1 if resume else n}")
    run.gate(f"{phase}: each handshake <= {max_ms} ms", (h.get("max") or 0) <= max_ms,
             f"max {h.get('max')} p95 {h.get('p95')} p50 {h.get('p50')} ms", f"<= {max_ms}")
    if resume:
        missed = (after.get("resume_missed") or 0) - (before.get("resume_missed") or 0)
        run.gate(f"{phase}: resumption taken every time", missed <= 1, f"{missed} full where resumed was asked", "<= 1 (the first)")
