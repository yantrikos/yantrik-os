#!/bin/bash
set -eu
PROXMOX="192.168.4.152"
CT=124
KEY="/home/yantrik/.ssh/id_deploy"
REGISTRY_ROOT="/var/www/bin"
VERSION="0.3.0"
TARGET="x86_64-unknown-linux-gnu"
COMP_DIR="/tmp/yantrik-components"

for tarball in "$COMP_DIR"/*.tar.zst; do
    name=$(basename "$tarball" .tar.zst)
    echo -n "Uploading $name... "
    ssh -i "$KEY" -o StrictHostKeyChecking=no "root@$PROXMOX" \
        "pct exec $CT -- mkdir -p $REGISTRY_ROOT/components/$name/$VERSION/" 2>/dev/null
    cat "$tarball" | ssh -i "$KEY" -o StrictHostKeyChecking=no "root@$PROXMOX" \
        "pct exec $CT -- tee $REGISTRY_ROOT/components/$name/$VERSION/$TARGET.tar.zst > /dev/null" 2>/dev/null
    echo "OK"
done

echo -n "Uploading nightly manifest... "
cat "$COMP_DIR/nightly.json" | ssh -i "$KEY" -o StrictHostKeyChecking=no "root@$PROXMOX" \
    "pct exec $CT -- tee $REGISTRY_ROOT/channels/nightly.json > /dev/null" 2>/dev/null
echo "OK"
echo "Done!"
