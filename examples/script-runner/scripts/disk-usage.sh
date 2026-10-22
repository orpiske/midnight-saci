#!/bin/sh
# Show the disk usage of one mount point. Changes nothing.
#
#   disk-usage.sh MOUNT
set -eu
exec df -P -- "$1"
