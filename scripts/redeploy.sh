#!/usr/bin/env bash
# Rebuild emem-server, restore cap_net_bind_service (so the binary can
# bind :443 as a non-root systemd user unit), and bounce the unit.
#
# Why this script exists: every `cargo build --release` replaces the
# emem-server binary, which silently drops the file capability set by
# `setcap`. Without it, `axum-server::bind(0.0.0.0:443)` returns
# `Permission denied (os error 13)` and the systemd user unit
# crash-loops at ~RestartSec=2s, eating CPU and producing thousands of
# log lines per hour. Hit this exact regression on 2026-04-30 (restart
# counter 1560 in 30 minutes after a smoke-test rebuild round) — this
# script makes the redeploy ritual atomic so the cap can't be forgotten.
#
# Run from anywhere; uses absolute paths.

set -euo pipefail

REPO=/home/ubuntu/emem
BIN="$REPO/target/release/emem-server"
UNIT=emem-server.service

cd "$REPO"

# Rebuild the embedded mdbook docs site so the binary picks up the
# latest `docs/*.md` content. `docs/book/` is baked into the binary via
# `include_dir!`; without this step a prod rebuild would ship stale docs.
# `mdbook` copies `book.toml` into its output because `src = "."` —
# drop it post-build so the responder doesn't leak its own build config.
echo "==> mdbook build (docs/)"
( cd "$REPO/docs" && mdbook build )
rm -f "$REPO/docs/book/book.toml"

# web/whitepaper-v2.html is generated from docs/whitepaper-v2.md and baked
# into the binary by include_str!, so it has to be regenerated BEFORE cargo
# build or the deploy ships a page that disagrees with its own source. v1
# kept the two as hand-written twins and they drifted; this is the fix.
echo "==> render whitepaper v2 (docs/whitepaper-v2.md -> web/whitepaper-v2.html)"
python3 "$REPO/scripts/render_whitepaper.py"

# The channel transcript is BAKED into the binary (web/channel.html), so it
# freezes at whatever date it was last generated unless this runs before the
# build. It sat frozen at 2026-07-21 for two days and a consumer agent had to
# tell us: their notes were in the store, visible via /v1/inbox, and absent
# from the page that is supposed to BE the record. Generating it here is the
# whole point of build_channel.py discovering peers from /v1/agents rather
# than carrying a hardcoded roster.
#
# Non-fatal on purpose: a deploy must not be blocked because the responder was
# briefly unreachable while regenerating a page. It warns and keeps the
# previous transcript instead.
echo "==> regenerate the agent channel (web/channel.html, docs/collaboration-log.md)"
python3 "$REPO/scripts/build_channel.py" || \
  echo "    ! channel regeneration failed; keeping the previous transcript"

echo "==> generate the static tool explorer (web/tools.html)"
python3 "$REPO/scripts/gen_tools_page.py" || \
  echo "    ! tool-explorer generation failed; keeping the previous page"

echo "==> cargo build --release -p emem-cli"
cargo build --release -p emem-cli

if [[ ! -x "$BIN" ]]; then
  echo "FAIL: $BIN missing after build" >&2
  exit 1
fi

echo "==> sudo setcap 'cap_net_bind_service=+ep' $BIN"
sudo setcap 'cap_net_bind_service=+ep' "$BIN"

echo "==> verifying cap"
caps=$(getcap "$BIN")
case "$caps" in
  *cap_net_bind_service*ep*) echo "ok: $caps" ;;
  *) echo "FAIL: cap not set on $BIN (got: $caps)" >&2; exit 2 ;;
esac

echo "==> systemctl --user restart $UNIT"
systemctl --user restart "$UNIT"

# Give axum-server a beat to bind, then sanity-check. Window doubled
# from 5×2s=10s to 10×2s=20s — cold-start with a fresh ACME challenge
# can easily take 15-30s on first boot of a redeployed binary, and
# false-failing the smoke check leaves an otherwise healthy server
# looking broken to the operator.
for i in 1 2 3 4 5 6 7 8 9 10; do
  sleep 2
  # Assert a real ANSWER, not a status code. The benchmark agent lost a run to
  # exactly this: their preflight checked /health, threw away the warmup
  # completion, and passed while every generation 500'd, so a full corpus of
  # empty answers recorded behind a green check. A deploy that only proves the
  # process is listening proves the cheapest thing.
  if curl -fsS -m 10 https://emem.dev/live >/dev/null 2>&1; then
    ans=$(curl -fsS -m 25 -X POST https://emem.dev/v1/recall \
            -H 'content-type: application/json' \
            -d '{"place":"Bengaluru","bands":["copdem30m.elevation_mean"]}' 2>/dev/null \
          | grep -o '"value":[0-9.]*' | head -1)
    if [ -n "$ans" ]; then
      echo "==> live and ANSWERING at https://emem.dev (after ${i}x2s): $ans"
      # ANSWERING IS NOT THE SAME AS DEPLOYED.
      #
      # Every check above this line is satisfied by the OLD binary: the port
      # is bound, TLS terminates, /v1/recall returns a number. A build that
      # failed, a restart that did not take, or a unit that came back on the
      # previous executable all reach this line and print success.
      #
      # build.rs has stamped the binary's own git commit since it was written
      # and nothing ever compared it to anything. This does, and it is the
      # only check here that can tell "the deploy worked" from "something is
      # serving".
      echo "==> verifying the responder is serving THIS commit"
      if python3 "$REPO/scripts/deploy_drift.py" --require-head; then
        # And that the BYTES a visitor receives are the bytes in this tree.
        # deploy_drift compares a self-reported commit, which is a claim about
        # provenance. The state that slipped past it today was simpler than any
        # stamp problem: a correct tree, every gate green, and a binary nobody
        # had rebuilt. Nothing here could see it, because every gate reads the
        # tree. It took an agent on the other side of the socket fetching the
        # page and finding the rule absent from the served CSS.
        echo "==> verifying the served pages are this tree's pages"
        if python3 "$REPO/scripts/served_matches_tree.py"; then
          exit 0
        fi
        echo "!!  the responder reports this commit and is serving different"
        echo "!!  bytes. The build or the restart did not take for at least one"
        echo "!!  baked page, and the stamp cannot see that."
        exit 1
      fi
      echo "!!  the deploy reported success and the responder is on another"
      echo "!!  commit. Nothing was rolled back: the previous binary is still"
      echo "!!  serving, which is why every check above it passed."
      exit 1
    fi
    echo "  listening but /v1/recall returned no value yet... ($i/10)"
    continue
  fi
  echo "  waiting for HTTPS to come up... ($i/10)"
done

echo "FAIL: https://emem.dev/health did not come up; tail logs:" >&2
journalctl --user -u "$UNIT" -n 20 --no-pager
exit 3
