#!/bin/sh
# Sync sources to the build server (see claude.md).
rsync -az --delete --exclude target --exclude .git --exclude dist -e "ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR" "$(dirname "$0")/" relkin@192.168.177.98:setup_maker/
