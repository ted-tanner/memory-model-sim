#!/bin/sh
set -e
cd "$(dirname "$0")"

for dir in riscv-programs/*/; do
	if [ -f "${dir}Makefile" ]; then
		( cd "$dir" && make )
	fi
done
