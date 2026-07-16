"""Signing, identity, and CLI for ememdev.

The signing math is mirrored three ways: the Rust responder
(`crates/emem-primitives/src/memory_acl.rs`), the LangChain adapter
(`emem-langmem`), and here. `FIXED_VECTOR` pins all of them to one known
(seed, verb, path, body) -> (pubkey, signature) so a mirror that drifts
fails here, not in production. The vector was computed independently
(raw blake3 + ed25519) and cross-checked against the signer; regenerating
it is a red flag unless the wire preimage genuinely changed.
"""

from __future__ import annotations

import base64
import json
import os
import stat

import pytest

from ememdev import signing
from ememdev.signing import EmemSigner, attester_preimage, body_hash

# Seed 00 01 .. 1f, body "hello emem", create verb. Anchoring constant.
FIXED_SEED = bytes(range(32))
FIXED_BODY = b"hello emem"
FIXED_PUBKEY_B32 = "aoqqpp7tzyil4hlq3umoos6atft6jvrqtosq2xy53sdgiesvgg4a"
FIXED_PUBKEY_SHORT = "aoqqpp7t"
FIXED_PATH = "/memories/by_attester/aoqqpp7t/note.md"
FIXED_SIG_B32 = (
    "ftodl5jzxvp27xinfzwxbcp3hzloedjumbrlgsu745xvpfitls6jb7viuyz2fgcj"
    "6lnpfu233jga62vkd2zyzmvg2hmione3ch44kcy"
)


def test_fixed_vector_pins_the_wire_signature() -> None:
    """The one test that keeps the three mirrors byte-identical."""
    signer = EmemSigner(FIXED_SEED)
    assert signer.pubkey_b32 == FIXED_PUBKEY_B32
    assert signer.pubkey_short == FIXED_PUBKEY_SHORT
    assert signer.namespace_root == f"/memories/by_attester/{FIXED_PUBKEY_SHORT}"
    block = signer.attester_block("create", FIXED_PATH, FIXED_BODY)
    assert block == {"pubkey_b32": FIXED_PUBKEY_B32, "sig_b32": FIXED_SIG_B32}


def test_wire_field_lengths_match_the_responder() -> None:
    """The responder requires exactly 52-char pubkey / 103-char sig."""
    signer = EmemSigner(FIXED_SEED)
    block = signer.attester_block("create", FIXED_PATH, FIXED_BODY)
    assert len(block["pubkey_b32"]) == 52
    assert len(block["sig_b32"]) == 103


def test_preimage_is_the_documented_construction() -> None:
    from blake3 import blake3

    bh = body_hash(FIXED_BODY)
    expected = blake3(
        b"emem.memory_write|create|" + FIXED_PATH.encode() + b"|" + bh
    ).digest()
    assert attester_preimage("create", FIXED_PATH, bh) == expected


def test_delete_body_hash_is_empty_blake3() -> None:
    from blake3 import blake3

    assert body_hash() == blake3(b"").digest()
    assert body_hash(b"") == blake3(b"").digest()


def test_rename_binds_destination_in_path_and_source_in_body() -> None:
    signer = EmemSigner(FIXED_SEED)
    old = f"{signer.namespace_root}/a.md"
    new = f"{signer.namespace_root}/b.md"
    block = signer.rename_attester_block(old, new)
    # Recompute: preimage path is the DESTINATION, body-hash is over the SOURCE.
    expected_pre = attester_preimage(
        "rename", new, signing.rename_body_hash(old)
    )
    expected_sig = signing.b32_nopad_lc(signer.sign_preimage(expected_pre))
    assert block["sig_b32"] == expected_sig
    # And it must NOT equal the naive create-shaped signature over `new`.
    naive = signer.attester_block("create", new, old.encode())
    assert block["sig_b32"] != naive["sig_b32"]


def test_owns_only_its_own_namespace() -> None:
    signer = EmemSigner(FIXED_SEED)
    assert signer.owns(f"{signer.namespace_root}/x.md")
    assert not signer.owns("/memories/by_attester/deadbeef/x.md")
    # A path outside by_attester/ has no owner.
    assert not signer.owns("/memories/shared/x.md")


def test_from_hex_round_trips() -> None:
    signer = EmemSigner.from_hex(FIXED_SEED.hex())
    assert signer.pubkey_b32 == FIXED_PUBKEY_B32
    assert signer.seed_bytes == FIXED_SEED


def test_string_key_is_rejected_with_a_hint() -> None:
    with pytest.raises(TypeError, match="from_hex"):
        EmemSigner(FIXED_SEED.hex())  # type: ignore[arg-type]


def test_repr_never_leaks_the_seed() -> None:
    signer = EmemSigner(FIXED_SEED)
    r = repr(signer)
    assert FIXED_SEED.hex() not in r
    assert signer.pubkey_short in r


# ── parity with the emem-langmem mirror, when it is importable ─────────


def test_matches_the_langmem_mirror() -> None:
    langmem_signing = pytest.importorskip("emem_langmem.signing")
    theirs = langmem_signing.EmemSigner(FIXED_SEED)
    ours = EmemSigner(FIXED_SEED)
    for verb in ("create", "str_replace", "insert", "delete"):
        assert ours.attester_block(verb, FIXED_PATH, FIXED_BODY) == theirs.attester_block(
            verb, FIXED_PATH, FIXED_BODY
        ), f"{verb} block drifted from the langmem mirror"
    assert ours.rename_attester_block(FIXED_PATH, FIXED_PATH + ".2") == (
        theirs.rename_attester_block(FIXED_PATH, FIXED_PATH + ".2")
    )


# ── identity ───────────────────────────────────────────────────────────


def test_load_or_create_mints_0600_and_reloads_stably(tmp_path) -> None:
    from ememdev.identity import load_or_create_signer, load_signer

    keyfile = tmp_path / "agent_ed25519.pem"
    first = load_or_create_signer(keyfile)
    assert keyfile.is_file()
    mode = stat.S_IMODE(keyfile.stat().st_mode)
    assert mode == 0o600, f"key file mode is {oct(mode)}, must be 0600"
    # Reloading returns the SAME identity, not a new one.
    again = load_signer(keyfile)
    assert again.pubkey_b32 == first.pubkey_b32


def test_load_signer_raises_when_absent(tmp_path) -> None:
    from ememdev.identity import load_signer

    with pytest.raises(FileNotFoundError):
        load_signer(tmp_path / "nope.pem")


def test_env_hex_seed_wins(tmp_path, monkeypatch) -> None:
    from ememdev.identity import load_or_create_signer

    monkeypatch.setenv("EMEM_AGENT_KEY", FIXED_SEED.hex())
    # Even with a keyfile present, the env hex is resolved first.
    (tmp_path / "agent_ed25519.pem").write_text("ignored")
    signer = load_or_create_signer()
    assert signer.pubkey_b32 == FIXED_PUBKEY_B32


def test_env_path_points_at_a_keyfile(tmp_path, monkeypatch) -> None:
    from ememdev.identity import load_or_create_signer

    keyfile = tmp_path / "k.pem"
    minted = load_or_create_signer(keyfile)
    monkeypatch.setenv("EMEM_AGENT_KEY", str(keyfile))
    resolved = load_or_create_signer()
    assert resolved.pubkey_b32 == minted.pubkey_b32


def test_env_home_moves_the_default_path(tmp_path, monkeypatch) -> None:
    from ememdev import identity

    monkeypatch.setenv("EMEM_HOME", str(tmp_path))
    monkeypatch.delenv("EMEM_AGENT_KEY", raising=False)
    assert identity.default_key_path() == tmp_path / "agent_ed25519.pem"
    signer = identity.load_or_create_signer()
    assert (tmp_path / "agent_ed25519.pem").is_file()
    assert identity.load_signer().pubkey_b32 == signer.pubkey_b32


def test_hex_seed_file_also_loads(tmp_path) -> None:
    from ememdev.identity import load_signer

    keyfile = tmp_path / "seed.hex"
    keyfile.write_text(FIXED_SEED.hex())
    assert load_signer(keyfile).pubkey_b32 == FIXED_PUBKEY_B32


# ── CLI ────────────────────────────────────────────────────────────────


def test_cli_whoami_reports_the_namespace(tmp_path, monkeypatch, capsys) -> None:
    from ememdev.__main__ import main

    monkeypatch.setenv("EMEM_HOME", str(tmp_path))
    monkeypatch.delenv("EMEM_AGENT_KEY", raising=False)
    rc = main(["whoami"])
    assert rc == 0
    out = json.loads(capsys.readouterr().out)
    assert out["namespace_root"].startswith("/memories/by_attester/")
    assert out["pubkey_short"] == out["pubkey_b32"][:8]


def test_cli_sign_emits_a_valid_block(tmp_path, monkeypatch, capsys) -> None:
    from ememdev.__main__ import main

    monkeypatch.setenv("EMEM_AGENT_KEY", FIXED_SEED.hex())
    rc = main(["sign", "--verb", "create", "--path", FIXED_PATH, "--body", "hello emem"])
    assert rc == 0
    out = json.loads(capsys.readouterr().out)
    assert out["attester"] == {"pubkey_b32": FIXED_PUBKEY_B32, "sig_b32": FIXED_SIG_B32}


def test_cli_sign_refuses_another_keys_namespace(monkeypatch) -> None:
    from ememdev.__main__ import main

    monkeypatch.setenv("EMEM_AGENT_KEY", FIXED_SEED.hex())
    with pytest.raises(SystemExit, match="another key's namespace"):
        main(["sign", "--path", "/memories/by_attester/deadbeef/x.md", "--body", "hi"])


def test_cli_sign_requires_a_body_for_create(monkeypatch) -> None:
    from ememdev.__main__ import main

    monkeypatch.setenv("EMEM_AGENT_KEY", FIXED_SEED.hex())
    with pytest.raises(SystemExit, match="needs a body"):
        main(["sign", "--verb", "create", "--path", FIXED_PATH])
