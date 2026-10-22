#!/bin/sh
# Report that the host is up. Changes nothing.
set -eu
echo "host: $(uname -n)"
echo "kernel: $(uname -sr)"
echo "status: healthy"
