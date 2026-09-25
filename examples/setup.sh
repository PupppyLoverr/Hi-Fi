#!/bin/bash
# Example worktree setup hook. Runs in the background inside the new worktree
# with HIFI_GROUP_ID, HIFI_TAB_ID, HIFI_WORKSPACE_NAME, HIFI_WORKSPACE_PATH set.
set -e
echo "setting up $HIFI_WORKSPACE_NAME at $HIFI_WORKSPACE_PATH"
echo "install deps here, e.g.: npm install / pip install -r requirements.txt"
