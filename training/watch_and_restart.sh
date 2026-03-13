#!/bin/bash
# Watch for checkpoint-500, then restart with batch_size=8

CHECKPOINT_DIR="c:/Users/sync/codes/yantrik-os/training/checkpoints"
OUTPUT_FILE="C:/Users/sync/AppData/Local/Temp/claude/c--Users-sync-codes-yantrik-os/tasks/b6es07lga.output"
LOG_FILE="c:/Users/sync/codes/yantrik-os/training/watcher.log"

echo "[$(date)] Watcher started. Waiting for checkpoint-500..." | tee "$LOG_FILE"

# Wait for checkpoint-500 directory to appear
while true; do
    if [ -d "$CHECKPOINT_DIR/checkpoint-500" ]; then
        echo "[$(date)] Checkpoint-500 found!" | tee -a "$LOG_FILE"
        break
    fi
    # Also check current step from output
    CURRENT_STEP=$(tail -1 "$OUTPUT_FILE" 2>/dev/null | tr '\r' '\n' | grep -oP '\d+/2559' | head -1 | cut -d/ -f1)
    if [ -n "$CURRENT_STEP" ] && [ "$CURRENT_STEP" -ge 510 ] 2>/dev/null; then
        echo "[$(date)] Step $CURRENT_STEP >= 510, checkpoint-500 should exist" | tee -a "$LOG_FILE"
        break
    fi
    sleep 60
done

# Wait a bit for checkpoint to finish writing
sleep 30

echo "[$(date)] Killing current training process..." | tee -a "$LOG_FILE"
# Find and kill the python training process
PID=$(tasklist.exe 2>/dev/null | grep "python" | awk '{print $2}' | head -1)
if [ -n "$PID" ]; then
    taskkill.exe //PID "$PID" //F 2>/dev/null
    echo "[$(date)] Killed PID $PID" | tee -a "$LOG_FILE"
fi

sleep 10

echo "[$(date)] Restarting with batch_size=8, resuming from checkpoint-500..." | tee -a "$LOG_FILE"
C:/Python312/python.exe "c:/Users/sync/codes/yantrik-os/training/train_qlora.py" \
    --batch-size 8 \
    --resume "$CHECKPOINT_DIR/checkpoint-500" \
    2>&1 | tee -a "$LOG_FILE"

echo "[$(date)] Training complete (or failed)." | tee -a "$LOG_FILE"
