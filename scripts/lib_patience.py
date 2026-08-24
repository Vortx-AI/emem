#!/usr/bin/env python3
"""Wait when the responder says wait. A 429 is not a finding.

    from lib_patience import patient

    with patient(urllib.request.Request(url), timeout=60) as r:
        body = r.read()

Why this exists
---------------
This repository's gates were red on commit after commit, and not one of the
failures was about the thing being checked. `parity.py` reported
`emem_change_attribution/base: refused differently: mcp=tool_error
rest=rate_limited`, which reads as two surfaces disagreeing and was in fact one
surface being throttled. `discovery_test.py` reported that a cold agent could
not obtain a fact to cite. Run alone, parity passes 15 of 15.

The responder publishes about 10 requests a second and refuses above it. Twenty
seven gates run back to back, each making dozens of calls, and the suite as a
whole walks straight through that ceiling. So the checks were failing because
of the limiter this project deliberately runs, and reporting it as a defect in
the surfaces they check. Four scripts already knew: build_channel.py paces
itself and its comment says why, having silently dropped 129 notes before it
did.

A 429 carries no information about correctness. It says the caller was early,
and the only correct response is to be later. Treating it as a verdict is the
same category error as treating a refused connection as a missing route.

What it does NOT do
-------------------
Hide a real refusal. After the attempts are spent the 429 is raised like any
other HTTPError, so a genuinely rate-limited surface still fails the gate that
found it, with the status intact. This buys patience, not silence.

It also does not touch any other status. A 404 and a 500 are answers about the
thing being checked and are returned immediately, unretried.
"""
import time
import urllib.error
import urllib.request

# Four attempts over about fifteen seconds. The window the responder enforces
# is per minute, so the point is to leave it rather than to outlast it, and a
# gate suite that pauses fifteen seconds is still a gate suite; one that waits
# a full minute per call is not.
ATTEMPTS = 4
BACKOFF_S = (1.0, 3.0, 8.0)


def patient(req, timeout=60, attempts=ATTEMPTS, **kw):
    """urlopen, but a 429 is waited out rather than returned as a result.

    Transparent on purpose: **kw goes straight through, because callers pass
    `context=` for a custom SSL context and a wrapper that silently drops it
    would change what is being verified. The first version did not, and
    mcp_host_compat.py stopped running entirely rather than running
    differently, which is the better of the two ways to be wrong.
    """
    last = None
    for i in range(attempts):
        try:
            return urllib.request.urlopen(req, timeout=timeout, **kw)
        except urllib.error.HTTPError as e:
            if e.code != 429:
                raise
            last = e
            if i + 1 < attempts:
                # Retry-After when the responder states one, our own schedule
                # when it does not. Believing the server over ourselves is the
                # whole point of the header.
                wait = BACKOFF_S[min(i, len(BACKOFF_S) - 1)]
                try:
                    ra = float(e.headers.get("Retry-After", "") or 0)
                    if 0 < ra <= 30:
                        wait = ra
                except (TypeError, ValueError):
                    pass
                time.sleep(wait)
    raise last
