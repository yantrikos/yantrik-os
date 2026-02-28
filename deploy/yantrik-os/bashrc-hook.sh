#!/bin/sh
# Yantrik Error Companion — log failed commands for proactive help.
# Source this from ~/.bashrc:
#   . /opt/yantrik/bashrc-hook.sh

__yantrik_log_cmd() {
    local ec=$?
    [ $ec -eq 0 ] && return
    local cmd
    cmd=$(HISTTIMEFORMAT='' history 1 | sed 's/^[ ]*[0-9]*[ ]*//')
    mkdir -p ~/.yantrik
    printf '%s\t%s\t%d\n' "$(date +%s)" "$cmd" "$ec" >> ~/.yantrik/cmd_log
}
PROMPT_COMMAND="__yantrik_log_cmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
