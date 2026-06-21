#!/usr/bin/env bash
# Fetch the real assets the engine needs (gitignored: large model + audio).
# - ggml-base.en.bin : whisper.cpp base English model (~142 MB)
# - jfk.wav          : the canonical JFK speech sample (real recording, ~344 KB)
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p models assets

MODEL="models/ggml-base.en.bin"
WAV="assets/jfk.wav"

if [[ ! -f "$MODEL" ]]; then
  echo "downloading $MODEL ..."
  curl -fsSL https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin -o "$MODEL"
fi
echo "model: $(du -h "$MODEL" | cut -f1)  $MODEL"

if [[ ! -f "$WAV" ]]; then
  echo "downloading $WAV ..."
  curl -fsSL https://github.com/ggerganov/whisper.cpp/raw/master/samples/jfk.wav -o "$WAV"
fi
echo "audio: $(du -h "$WAV" | cut -f1)  $WAV"
echo "done."
