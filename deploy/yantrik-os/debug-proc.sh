#!/bin/sh
# Debug yantrik-ui process state

# Find PID
UI_PID=$(ps aux | grep '[y]antrik-ui' | head -1 | awk '{print $1}')
echo "yantrik-ui PID: $UI_PID"

if [ -z "$UI_PID" ]; then
    echo "yantrik-ui NOT RUNNING!"
    exit 1
fi

echo
echo "=== Environment ==="
cat /proc/$UI_PID/environ 2>/dev/null | tr '\0' '\n' | sort

echo
echo "=== Open file descriptors ==="
ls -la /proc/$UI_PID/fd/ 2>/dev/null | head -20

echo
echo "=== Memory usage ==="
cat /proc/$UI_PID/status 2>/dev/null | grep -E "VmRSS|VmSize|Threads"

echo
echo "=== CPU time ==="
ps -p $UI_PID -o pid,etime,cputime,rss 2>/dev/null || ps aux | grep $UI_PID | grep -v grep
