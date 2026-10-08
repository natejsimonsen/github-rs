#!/usr/bin/env bash
# Set the app's version in Cargo.toml and Cargo.lock. The release pipeline
# runs this; Nix and Flox read the version from Cargo.toml.
set -euo pipefail
v="${1:?usage: set-version.sh <version>}"
perl -0pi -e "s/^version = \"[^\"]+\"/version = \"$v\"/m" Cargo.toml
perl -0pi -e "s/(name = \"github-prs\"\nversion = \")[^\"]+/\${1}$v/" Cargo.lock
grep -q "^version = \"$v\"" Cargo.toml
grep -A1 '^name = "github-prs"$' Cargo.lock | grep -q "^version = \"$v\""
