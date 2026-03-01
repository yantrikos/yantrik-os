#!/bin/bash
# Quick SSH helper for Yantrik VM
sshpass -p root ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o ServerAliveInterval=3 -o ServerAliveCountMax=2 -p 2222 root@localhost "$@"
