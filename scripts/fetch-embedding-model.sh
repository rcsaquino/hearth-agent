#!/usr/bin/env bash
# Fetch the local Bekko embedding model used by hybrid memory recall.
# The model is deliberately not committed. This script is the only network step.
set -euo pipefail

home="${HEARTH_HOME:-$HOME/.hearth}"
directory="$home/models/bekko-embedding-v1-a8m"
revision="c721113d59a1d91b447450324f51c4b3332c924a"
base="https://huggingface.co/hotchpotch/bekko-embedding-v1-a8m/resolve/$revision"
model_sha="96d8cc6199e96357b21b2fb12f6d7ffd2d4abc7b182fe94b5468fbd6dc819af7"
tokenizer_sha="8bd47075711f75a143d1b78e01a41cc65c1c591b00d3cfeffc23db07adce1392"

mkdir -p "$directory"

fetch() {
    local url="$1"
    local path="$2"
    local expected="$3"
    if [ -f "$path" ] && [ "$(sha256sum "$path" | cut -d' ' -f1)" = "$expected" ]; then
        echo "ok  $path"
        return
    fi
    local temporary="$path.tmp"
    echo "get $url"
    curl --fail --location --retry 3 --output "$temporary" "$url"
    local actual
    actual="$(sha256sum "$temporary" | cut -d' ' -f1)"
    if [ "$actual" != "$expected" ]; then
        rm -f "$temporary"
        echo "checksum mismatch for $path" >&2
        exit 1
    fi
    mv "$temporary" "$path"
    echo "ok  $path"
}

fetch "$base/tokenizer.json" "$directory/tokenizer.json" "$tokenizer_sha"
fetch "$base/onnx/model.onnx" "$directory/model.onnx" "$model_sha"
echo "embedding model ready: $directory"
