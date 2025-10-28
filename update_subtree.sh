#!/bin/bash
set -e  # exit on any error

# Resolve directory of this script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

git stash
bash "$SCRIPT_DIR/update_subtree_no_stash.sh"
git stash pop