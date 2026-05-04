#!/usr/bin/env bash
# Download BAAI/bge-base-en-v1.5 (Xenova ONNX mirror) into the
# convention directory the topic router expects:
#
#   $EMEM_DATA/models/bge-base-en-v1.5/
#     ├── model.onnx          (~435 MB, the BERT-base ONNX)
#     └── tokenizer.json      (~700 KB, WordPiece + special tokens)
#
# Both files come from huggingface.co/Xenova/bge-base-en-v1.5/main.
# Run once per host. The ort backend reads from this directory at
# startup; no hf-hub call is made at runtime.
#
# Override the destination by setting EMEM_TOPIC_MODEL_DIR before
# running emem-server (or in the systemd unit env).

set -euo pipefail
DEST="${EMEM_TOPIC_MODEL_DIR:-${EMEM_DATA:-./var/emem}/models/bge-base-en-v1.5}"
mkdir -p "$DEST"
cd "$DEST"

base="https://huggingface.co/Xenova/bge-base-en-v1.5/resolve/main"
echo "==> tokenizer.json"
[[ -f tokenizer.json ]] || curl -L -o tokenizer.json "$base/tokenizer.json"
echo "==> model.onnx (~435 MB, may take a minute)"
[[ -f model.onnx ]] || curl -L -o model.onnx "$base/onnx/model.onnx"

echo
echo "installed at: $DEST"
ls -la "$DEST"
