#!/usr/bin/env bash
# 完整验证：跨平台质量检查 + 固定平台证据链。
# 用法: scripts/verify-all.sh        （全部）
#       scripts/verify-all.sh ch07   （证据链只跑匹配的章）
set -euo pipefail
cd "$(dirname "$0")/.."

scripts/verify-portable.sh
scripts/verify-evidence.sh "${1:-}"
