"""`python -m ememdev` / `ememdev` — sign and write emem memory notes.

Built for an agent that speaks to emem over MCP and only needs the one
thing the MCP client cannot do for it: produce the ed25519 `attester`
block a memory write requires. Shell out to `ememdev sign` and pipe the
block into your own request, or let `ememdev write` sign and POST it in
one step.

    ememdev whoami
    ememdev sign  --verb create --path /memories/by_attester/<you>/n.md --stdin
    ememdev write --path /memories/by_attester/<you>/n.md --body-file n.md

The identity comes from the standard location (see `ememdev.identity`):
`EMEM_AGENT_KEY`, then `~/.emem/agent_ed25519.pem`, minted on first
`write`/`whoami` if absent. `sign` never mints — it fails if there is no
identity, so a script cannot accidentally sign under a throwaway key.
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import Optional


def _read_body(args: argparse.Namespace) -> bytes:
    sources = [args.body is not None, bool(args.body_file), args.stdin]
    if sum(sources) > 1:
        raise SystemExit("give at most one of --body, --body-file, --stdin")
    if args.body is not None:
        return args.body.encode("utf-8")
    if args.body_file:
        with open(args.body_file, "rb") as fh:
            return fh.read()
    if args.stdin:
        return sys.stdin.buffer.read()
    return b""


def _mcp_endpoint(base_url: str) -> str:
    return base_url.rstrip("/") + "/mcp"


def _post_memory_create(
    base_url: str, path: str, file_text: str, kind: str, attester: dict
) -> dict:
    """POST a signed memory_create over MCP JSON-RPC and return the tool
    result. Handles both a plain JSON response and an SSE `data:` stream,
    which is what the Streamable-HTTP endpoint may emit."""
    import httpx

    payload = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "memory_create",
            "arguments": {
                "path": path,
                "file_text": file_text,
                "kind": kind,
                "attester": attester,
            },
        },
    }
    headers = {
        "content-type": "application/json",
        "accept": "application/json, text/event-stream",
    }
    with httpx.Client(timeout=90.0) as http:
        resp = http.post(_mcp_endpoint(base_url), json=payload, headers=headers)
    resp.raise_for_status()
    envelope = _parse_jsonrpc(resp.text)
    if "error" in envelope:
        raise SystemExit(f"memory_create failed: {json.dumps(envelope['error'])}")
    result = envelope.get("result", {})
    if result.get("isError"):
        text = _first_text(result)
        raise SystemExit(f"memory_create rejected: {text}")
    return _decode_tool_json(result)


def _parse_jsonrpc(body: str) -> dict:
    """Extract the JSON-RPC envelope from a plain body or an SSE stream."""
    body = body.strip()
    last: Optional[dict] = None
    for line in body.splitlines():
        line = line.strip()
        if line.startswith("data:"):
            line = line[len("data:") :].strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(obj, dict) and ("result" in obj or "error" in obj):
            last = obj
    if last is None:
        raise SystemExit(f"no JSON-RPC envelope in response: {body[:300]}")
    return last


def _first_text(result: dict) -> str:
    for item in result.get("content", []):
        if item.get("type") == "text":
            return item.get("text", "")
    return json.dumps(result)[:300]


def _decode_tool_json(result: dict) -> dict:
    """The memory_create tool returns its receipt as JSON text inside the
    MCP content block; unwrap it, falling back to the raw text."""
    text = _first_text(result)
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return {"raw": text}


def _load_signer(create_ok: bool):
    from .identity import load_or_create_signer, load_signer

    return load_or_create_signer() if create_ok else load_signer()


def _cmd_whoami(args: argparse.Namespace) -> int:
    signer = _load_signer(create_ok=True)
    print(
        json.dumps(
            {
                "pubkey_b32": signer.pubkey_b32,
                "pubkey_short": signer.pubkey_short,
                "namespace_root": signer.namespace_root,
            },
            indent=2,
        )
    )
    return 0


def _build_attester(signer, args: argparse.Namespace) -> dict:
    verb = args.verb
    if verb == "rename":
        if not args.old_path:
            raise SystemExit("rename needs --old-path (the source) and --path (the destination)")
        return signer.rename_attester_block(args.old_path, args.path)
    body = _read_body(args)
    if verb == "delete" and body:
        raise SystemExit("delete carries no body; drop --body/--body-file/--stdin")
    if verb in ("create", "str_replace", "insert") and not body and not args.allow_empty:
        raise SystemExit(
            f"{verb} needs a body (the whole post-edit file for str_replace/insert). "
            "Pass --body/--body-file/--stdin, or --allow-empty to sign an empty body."
        )
    return signer.attester_block(verb, args.path, body)


def _cmd_sign(args: argparse.Namespace) -> int:
    signer = _load_signer(create_ok=False)
    if not signer.owns(args.path) and args.path.startswith("/memories/by_attester/"):
        raise SystemExit(
            f"path {args.path} is under another key's namespace; this identity owns "
            f"{signer.namespace_root}/..."
        )
    attester = _build_attester(signer, args)
    print(json.dumps({"attester": attester}, indent=2))
    return 0


def _cmd_write(args: argparse.Namespace) -> int:
    signer = _load_signer(create_ok=True)
    if not signer.owns(args.path) and args.path.startswith("/memories/by_attester/"):
        raise SystemExit(
            f"path {args.path} is under another key's namespace; this identity owns "
            f"{signer.namespace_root}/..."
        )
    body = _read_body(args)
    if not body and not args.allow_empty:
        raise SystemExit("write needs a body; pass --body/--body-file/--stdin or --allow-empty")
    file_text = body.decode("utf-8")
    attester = signer.attester_block("create", args.path, body)
    result = _post_memory_create(args.base_url, args.path, file_text, args.kind, attester)
    print(json.dumps(result, indent=2))
    return 0


def _add_body_args(p: argparse.ArgumentParser) -> None:
    p.add_argument("--body", help="body as a literal string")
    p.add_argument("--body-file", help="read the body from a file")
    p.add_argument("--stdin", action="store_true", help="read the body from stdin")
    p.add_argument(
        "--allow-empty",
        action="store_true",
        help="permit an empty body (rarely what you want)",
    )


def main(argv: Optional[list] = None) -> int:
    parser = argparse.ArgumentParser(
        prog="ememdev",
        description="Sign and write emem memory notes with a standard ed25519 identity.",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("whoami", help="print this identity's pubkey and namespace")

    p_sign = sub.add_parser(
        "sign", help="emit the attester block for one write (no network)"
    )
    p_sign.add_argument(
        "--verb",
        default="create",
        choices=["create", "str_replace", "insert", "delete", "rename"],
    )
    p_sign.add_argument("--path", required=True, help="the memory path being written")
    p_sign.add_argument("--old-path", help="rename only: the source path")
    _add_body_args(p_sign)

    p_write = sub.add_parser(
        "write", help="sign a create and POST it to /mcp in one step"
    )
    p_write.add_argument("--path", required=True)
    p_write.add_argument("--kind", default="resource")
    p_write.add_argument("--base-url", default="https://emem.dev")
    _add_body_args(p_write)

    args = parser.parse_args(argv)
    if args.command == "whoami":
        return _cmd_whoami(args)
    if args.command == "sign":
        return _cmd_sign(args)
    if args.command == "write":
        return _cmd_write(args)
    parser.error(f"unknown command {args.command!r}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
