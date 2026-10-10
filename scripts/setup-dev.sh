#!/usr/bin/env bash
# Sekali setelah clone: aktifkan hook repo.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
git config core.hooksPath .githooks
chmod +x .githooks/* scripts/*.sh
echo "core.hooksPath = .githooks"
echo "Identitas commit: $(git config user.name) <$(git config user.email)>"
