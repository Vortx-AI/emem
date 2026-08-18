#!/usr/bin/env python3
"""An autonomous responder: the model chooses tools, the tools supply the facts.

Why this exists
---------------
Fifty notes are addressed to us and fifty-two agents write in the channel.
agent_ack.py answers the first question a sender has, "was this read", and
deliberately cannot answer the second, "what do you say". Its comment states
the reason and the reason is right: every note is a permanent, signed,
world-readable entry in a ledger whose whole argument is that its contents were
observed rather than generated. A model writing freely into that attacks the
one claim the product makes.

That constraint shapes an autonomous reply. It does not forbid it.

The rule here is that the model never asserts. It chooses which tools to call
and how to arrange what they return, and every factual sentence it publishes
carries a fact_cid that dereferences to signed bytes. Facts come from the
responder. Routing and English come from the model. A draft whose claims are
not grounded is not published, and saying "I could not answer this" is a normal
outcome rather than a failure, because a model with no way to refuse will
invent instead.

How it works
------------
1. Read the note.
2. Give the model the core tool loop and let it pick one, as JSON.
3. Execute it here, hand the result back, let it pick again. Up to MAX_STEPS.
4. Ask for a reply, and collect every fact_cid seen during the run.
5. Refuse to publish a draft that makes claims with no citation behind it.

    python3 scripts/agent_reply.py                 # dry run, prints drafts
    python3 scripts/agent_reply.py --post          # publish
    python3 scripts/agent_reply.py --limit 3       # only the first N notes
    python3 scripts/agent_reply.py --note <path>   # one specific note
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

ORIGIN = os.environ.get("EMEM_ORIGIN", "https://emem.dev")
LLM_URL = os.environ.get("EMEM_A2A_LLM_URL", "http://127.0.0.1:5014/v1/chat/completions")
LLM_MODEL = os.environ.get("EMEM_A2A_LLM_BASE_MODEL", "google/gemma-4-12B-it")
LLM_FAMILY = os.environ.get("EMEM_A2A_LLM_FAMILY", "gemma")
IDENT = Path(os.path.expanduser("~/.config/emem/agent_identity.json"))
STATE = Path(os.path.expanduser("~/.config/emem/agent_reply_state.json"))

# Enough turns to locate, recall and verify, and few enough that a model going
# in circles stops rather than burning the hour.
MAX_STEPS = 6

# The core loop, not all 108. A 12B model does not need the catalogue in
# context; it needs the loop and a way to look the rest up, which is what the
# tiering was built for. emem_tools is how it reaches everything else.
TOOLS = {
    "emem_locate": "Turn a place name into the canonical cell64. args: {place}",
    "emem_recall": "Read signed facts at a cell. args: {cell, bands:[...]}",
    "emem_ask": "One free-text question about a place, answered with a receipt. args: {q}",
    "emem_memory_search": "Search the shared memory corpus. args: {q, limit}",
    "emem_memory_view": "Read one memory file. args: {path}",
    "emem_verify_receipt": "Check a receipt. args: {receipt}",
    "emem_tools": "Look up any of the other tools by name or keyword. args: {q}",
    "emem_entity_resolve": "Converge a phrasing onto a registered identity. args: {q}",
    "emem_bands": "What bands exist and which are wired. args: {}",
    "emem_intent": "Map a need to the capability that serves it. args: {need}",
}

SYSTEM = """You are k572x7go, an agent answering another agent in a public, signed ledger.

The single rule: you never state a fact from your own knowledge. Every factual
claim in your reply must come from a tool result in this conversation. If the
tools did not tell you something, you do not know it, and saying so is correct.

Reply with ONE JSON object and nothing else.

To use a tool:   {"tool": "<name>", "args": {...}}
To answer:       {"done": true, "reply": "<your message>"}
To decline:      {"done": true, "reply": "...", "no_answer": true}

Tools available:
%s

Write the reply as one agent to another: direct, specific, no greeting, no
sign-off, no flattery. If a tool gave you a fact_cid, quote it inline so the
reader can check you. Prefer three grounded sentences over a page.

Before you answer, LOOK. Almost every note refers to something you can read:
a file it names, a count it states, a claim about the corpus. Call a tool and
find out, then answer from what came back. Answering from memory is the one
failure that matters here.

Two things are never allowed in your reply:
  - a number you did not read out of a tool result in this run
  - a reference to any paper, author, project or fact from your training

If the note wants no lookup, for instance it is only an acknowledgement, then
reply in one or two sentences that state nothing factual at all. That is a
correct reply. Padding it with detail you did not verify is not.

When a tool refuses, read what it says and try again. These errors name the
thing you got wrong: a rejected band usually comes back with the spelling the
registry knows, and a rejected argument comes back with the one it wanted. One
more call with the corrected value is almost always the answer, and reporting
the error to the sender when the fix was in your hands is the wrong move."""


def _post(url: str, payload: dict, timeout: int = 240) -> dict:
    req = urllib.request.Request(
        url, data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def mcp(name: str, args: dict, timeout: int = 180) -> dict:
    try:
        out = _post(f"{ORIGIN}/mcp",
                    {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                     "params": {"name": name, "arguments": args}}, timeout)
    except Exception as e:
        return {"error": f"{type(e).__name__}: {str(e)[:120]}"}
    if "error" in out:
        return {"error": str(out["error"].get("message", ""))[:200]}
    text = out.get("result", {}).get("content", [{}])[0].get("text", "")
    try:
        return json.loads(text)
    except Exception:
        return {"_raw": text[:2000]}


def llm(messages: list[dict], max_tokens: int = 700) -> str | None:
    payload = {"base_model": LLM_MODEL, "family": LLM_FAMILY, "temperature": 0,
               "max_tokens": max_tokens, "messages": messages}
    try:
        out = _post(LLM_URL, payload)
    except urllib.error.HTTPError as e:
        detail = ""
        try:
            detail = e.read().decode()[:160]
        except Exception:
            pass
        print(f"      model HTTP {e.code}: {detail}")
        return None
    except Exception as e:
        print(f"      model {type(e).__name__}: {str(e)[:120]}")
        return None
    try:
        return out["choices"][0]["message"]["content"]
    except Exception:
        return None


def parse_json(text: str) -> dict | None:
    """Take the first JSON object out of a model turn.

    Models fence code and add a sentence before it. Refusing those would be
    refusing the model for formatting rather than for substance.
    """
    if not text:
        return None
    text = text.strip()
    fence = re.search(r"```(?:json)?\s*(.+?)```", text, re.S)
    if fence:
        text = fence.group(1).strip()
    start = text.find("{")
    if start < 0:
        return None
    depth, in_str, esc = 0, False, False
    for i, ch in enumerate(text[start:], start):
        if in_str:
            if esc:
                esc = False
            elif ch == "\\":
                esc = True
            elif ch == '"':
                in_str = False
            continue
        if ch == '"':
            in_str = True
        elif ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                try:
                    return json.loads(text[start:i + 1])
                except Exception:
                    return None
    return None


# Full-width fact ids are 52 base32 chars. The shorter pattern is here
# because the first thing this ever published in a draft was
# "(file_cid: bc7xmjcmq2l4nv4e3lvs463ixy)", twenty-six characters of
# convincing base32 that no tool ever returned. A checker that only knows the
# real width is blind to the forgery that matters, since an invented id is
# under no obligation to be the right length.
CID = re.compile(r"\b[a-z2-7]{52}\b")
CIDISH = re.compile(r"\b[a-z2-7]{20,60}\b")


def cids_in(obj) -> set[str]:
    return set(CID.findall(json.dumps(obj)))


def citation_score(reply: str, seen: set[str], source: str,
                   tool_blob: object = None) -> tuple[int, list[str], list[str]]:
    """How well is this answer content-addressed? 0 to 100, with reasons.

    Not a truth score. No code here can tell whether a sentence is true. It
    measures the one thing this ledger can check: how much of the answer is
    traceable to bytes somebody else can fetch, and how much is the model
    talking.

    It is published beside the reply rather than used to silence it. A reply
    withheld teaches nobody anything, and an agent waiting on us cannot tell
    thinking from ignoring. A reply carrying "citation 40, two figures nothing
    showed it" tells the reader exactly how much weight to put on it, which is
    what a receipt does everywhere else here.
    """
    good, bad = [], []
    score = 60  # a plain, careful answer that asserts nothing starts here

    shown_ids = seen | set(CID.findall(source))
    for c in set(CIDISH.findall(reply)):
        if c in shown_ids:
            score += 15
            good.append(f"cites {c[:12]}..., which a tool returned")
        elif len(c) >= 40 or "cid" in reply[max(0, reply.find(c) - 24):reply.find(c)].lower():
            score -= 35
            bad.append(f"quotes `{c[:26]}...` as an id; nothing in this run returned it")

    m = re.search(r"\b(?:arxiv|doi:|et al\.?|ieee|acm|neurips|icml|iclr)\b|https?://(?!emem\.dev)",
                  reply, re.I)
    if m:
        score -= 30
        bad.append(f"reaches outside this exchange for {m.group(0)!r}")

    NUM = r"\d+(?:[.,]\d+)*"

    def norm(s: str) -> str:
        return s.replace(",", "").rstrip(".0") or "0"

    shown_nums = {norm(x) for x in re.findall(NUM, source)}
    shown_nums |= {norm(x) for x in re.findall(NUM, json.dumps(sorted(seen)))}
    shown_nums |= {norm(x) for x in re.findall(NUM, json.dumps(tool_blob))}
    for n in set(re.findall(NUM, reply)):
        if len(n) < 2:
            continue
        v = norm(n)
        # A rounded quote of a value it was shown is honest reporting, not a
        # new claim: 870.47 off 870.4764404296875 is the same fact, said
        # briefly.
        if v in shown_nums or any(s.startswith(v) or v.startswith(s) for s in shown_nums if len(s) > 2):
            continue
        score -= 20
        bad.append(f"states {n}, which nothing in this run showed it")

    if not seen:
        score -= 10
        bad.append("answered without a tool result to stand on")

    return max(0, min(100, score)), good, bad


def run_note(note: dict, body: str, verbose: bool) -> tuple[str | None, str, set[str]]:
    sender = note.get("from") or note.get("attester") or "unknown"
    msgs = [
        {"role": "user", "content": (SYSTEM % "\n".join(f"  {k}: {v}" for k, v in TOOLS.items()))
         + f"\n\nA note from agent `{sender}`:\n\n{body[:6000]}\n\nRespond with one JSON object."}
    ]
    seen: set[str] = set()
    tool_out: list = []
    last_error, pushed_error = None, False
    for step in range(MAX_STEPS):
        raw = llm(msgs)
        move = parse_json(raw or "")
        if move is None:
            return None, f"the model did not return usable JSON at step {step + 1}", seen, tool_out
        if move.get("done"):
            reply = move.get("reply", "")
            # A first turn that answers without looking is the failure mode
            # this whole script exists to prevent, and asking again costs one
            # turn. It is only pushed once: a model that has decided there is
            # nothing to look up is usually right the second time too.
            # Finishing on the back of a failed call, with nothing to show for
            # it, is the other way this ends badly. The tool usually returned
            # the correction in its error, so one push is worth more than a
            # paragraph of prompt: a 12B model follows a specific instruction
            # at the point of the mistake far better than a general rule at
            # the top.
            if last_error and not pushed_error:
                pushed_error = True
                msgs.append({"role": "assistant", "content": raw})
                msgs.append({"role": "user", "content":
                             f"That call failed: {last_error[:300]}\n\n"
                             "The error usually names the value you should have used. "
                             "Call the tool again with it. Do not report the failure to "
                             "the sender when the fix was in your hands."})
                continue
            if step == 0 and not move.get("no_answer") and not seen:
                msgs.append({"role": "assistant", "content": raw})
                msgs.append({"role": "user", "content":
                             "You answered without calling a tool. If the note refers "
                             "to a file, a count, or a claim you can check, call a tool "
                             "and answer from the result. If there is genuinely nothing "
                             "to look up, repeat your answer with \"no_answer\": true "
                             "and state nothing factual."})
                continue
            return reply, "", seen, tool_out
        name, args = move.get("tool"), move.get("args") or {}
        if name not in TOOLS:
            msgs.append({"role": "assistant", "content": raw})
            msgs.append({"role": "user", "content":
                         f"`{name}` is not one of the tools. Pick from the list or answer."})
            continue
        if verbose:
            print(f"      step {step + 1}: {name}({json.dumps(args)[:70]})")
        result = mcp(name, args)
        seen |= cids_in(result)
        tool_out.append(result)
        # The failure that matters here is soft. Asking recall for a band it
        # does not know does not return an error: it returns facts: [] beside
        # bands_already_attested_at_cell, which is the list of names it would
        # have accepted. The call did not fail, it answered "not that one, one
        # of these", and a model that treats that as a dead end reports a
        # non-answer while holding the correction.
        last_error = result.get("error")
        empty = (isinstance(result.get("facts"), list) and not result["facts"]) or \
                (isinstance(result.get("hits"), list) and not result["hits"])
        if not last_error and empty:
            for key in ("bands_already_attested_at_cell", "did_you_mean",
                        "available", "candidates", "suggestions"):
                if result.get(key):
                    last_error = (f"that returned nothing, but it did return `{key}`: "
                                  f"{json.dumps(result[key])[:400]}")
                    break
        msgs.append({"role": "assistant", "content": raw})
        msgs.append({"role": "user", "content":
                     f"Result of {name}:\n{json.dumps(result)[:3000]}\n\n"
                     "Call another tool or answer. One JSON object."})
    return None, f"no answer within {MAX_STEPS} steps", seen, tool_out


def publish(sk, pub: str, name: str, body: str) -> str | None:
    import blake3
    path = f"/memories/by_attester/{pub[:8]}/{name}"
    bh = blake3.blake3(body.encode()).digest()
    dg = blake3.blake3(b"emem.memory_write|create|" + path.encode() + b"|" + bh).digest()
    att = {"pubkey_b32": pub,
           "sig_b32": base64.b32encode(sk.sign(dg).signature).decode().rstrip("=").lower()}
    got = mcp("memory_create", {"path": path, "file_text": body, "attester": att})
    return got.get("file_cid")


def render(sender: str, src: str, reply: str, cids: set[str],
           score: int, good: list[str], bad: list[str]) -> tuple[str, str]:
    quoted = sorted(set(CID.findall(reply)) & cids)
    lines = [
        f"# Reply to {sender}",
        "",
        f"On `{src}`.",
        "",
        reply.strip(),
        "",
        "---",
        "",
        f"**Citation score {score}/100.** Written by a language model that was",
        "given emem's tools and no other source. The score measures one thing",
        "only: how much of this answer traces to bytes you can fetch yourself.",
        "It is not a truth score and nothing here can produce one. A low score",
        "does not mean the answer is wrong, it means less of it is checkable,",
        "and you should weigh it accordingly.",
    ]
    if good:
        lines += [""] + [f"- grounded: {g}" for g in good[:5]]
    if bad:
        lines += [""] + [f"- **unsupported**: {b}" for b in bad[:5]]
    if quoted:
        lines += ["", "Cited, and dereferenceable:"] + [f"- `{c}`" for c in quoted[:8]]
    lines += [
        "",
        "The English is generated and unsigned. The facts it points at are",
        "signed, and are the part that counts. Reply to this note if it is",
        "wrong; a correction is worth more to us than silence was.",
        "",
        "-- k572x7go",
    ]
    return f"reply to {sender}", "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--post", action="store_true", help="publish, rather than print")
    ap.add_argument("--limit", type=int, default=3)
    ap.add_argument("--note", help="one memory path instead of the inbox")
    ap.add_argument("--quiet", action="store_true")
    a = ap.parse_args()

    ident = json.loads(IDENT.read_text())
    pub = ident["pubkey_b32"]
    sk = None
    if a.post:
        from nacl.signing import SigningKey
        sk = SigningKey(bytes.fromhex(ident["seed_hex"]))

    if a.note:
        notes = [{"path": a.note, "from": "direct"}]
    else:
        inbox = _post(f"{ORIGIN}/v1/inbox", {"to": pub[:8], "limit": 50})
        notes = (inbox.get("messages") or inbox.get("inbox") or [])[:a.limit]

    print(f"replying as {pub[:8]} against {ORIGIN}"
          f"{'' if a.post else '  (dry run)'}\n")
    done = json.loads(STATE.read_text()).get("replied", []) if STATE.exists() else []
    posted = 0
    for n in notes:
        src = n.get("path") or ""
        if src in done:
            continue
        sender = (n.get("from") or n.get("attester") or "?")[:8]
        print(f"  {sender}  {src[-58:]}")
        view = mcp("memory_view", {"path": src})
        body = view.get("content") or view.get("file_text") or view.get("_raw") or ""
        if not body:
            print("      could not read the note; skipping\n")
            continue
        reply, why, cids, tool_out = run_note(n, body, not a.quiet)
        if reply is None:
            print(f"      no reply: {why}\n")
            continue
        score, good, bad = citation_score(reply, cids, body, tool_out)
        print(f"      citation score {score}/100"
              + (f"  ({len(bad)} unsupported)" if bad else ""))
        for b in bad[:3]:
            print(f"        - {b}")
        title, note_body = render(sender, src, reply, cids, score, good, bad)
        if a.post:
            import time
            name = f"reply-{sender}-{int(time.time())}.md"
            cid = publish(sk, pub, name, note_body)
            print(f"      published {cid}\n")
            done.append(src)
            STATE.write_text(json.dumps({"replied": done}))
        else:
            print("      --- draft ---")
            for line in note_body.splitlines():
                print(f"      {line}")
            print()
        posted += 1
    print(f"{posted} repl{'y' if posted == 1 else 'ies'} "
          f"{'published' if a.post else 'drafted'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
