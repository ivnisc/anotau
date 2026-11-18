#!/bin/bash
# Instalación o actualización de anotau-server como servicio de systemd.
# Uso: sudo bash ~/anotau-install/install.sh   (idempotente: se puede repetir)
set -euo pipefail
cd "$(dirname "$0")"
[ "$(id -u)" -eq 0 ] || { echo "Ejecuta con sudo." >&2; exit 1; }
sha256sum --check --quiet SHA256SUMS
echo "== usuario del servicio"
id anotau >/dev/null 2>&1 || useradd --system --home-dir /var/lib/anotau --no-create-home \
    --shell /usr/sbin/nologin anotau
echo "== directorios y configuración"
install -d -m 0755 -o root -g root /etc/anotau
install -d -m 0700 -o anotau -g anotau /etc/anotau/tls /var/lib/anotau /var/backups/anotau
install -m 0640 -o root -g anotau environment /etc/anotau/environment
install -m 0644 -o anotau -g anotau server.crt /etc/anotau/tls/server.crt
install -m 0600 -o anotau -g anotau server.key /etc/anotau/tls/server.key
echo "== binario y consola"
# respaldo antes de cambiar el binario: el arranque migra la base si hay versión nueva
if systemctl is-active --quiet anotau; then
    sh anotau-admin --backup || { echo "Falló el respaldo; no se actualiza." >&2; exit 1; }
fi
if [ -f /usr/local/bin/anotau-server ]; then
    cp -p /usr/local/bin/anotau-server /usr/local/bin/anotau-server.anterior
fi
install -m 0755 -o root -g root anotau-server /usr/local/bin/anotau-server
install -m 0755 -o root -g root anotau-admin /usr/local/bin/anotau-admin
echo "== servicio"
install -m 0644 -o root -g root anotau.service /etc/systemd/system/anotau.service
systemd-analyze verify /etc/systemd/system/anotau.service
systemctl daemon-reload
systemctl enable anotau >/dev/null
systemctl restart anotau
echo "== firewall"
ufw allow OpenSSH >/dev/null
ufw allow from 10.0.0.0/8 to any port 8443 proto tcp comment 'anotau' >/dev/null
ufw status | grep -q "Status: active" || ufw --force enable
ufw status verbose
echo "== comprobación"
for _ in 1 2 3 4 5 6 7 8 9 10; do
    if curl -fsS --cacert ca.crt "https://$(hostname -I | cut -d" " -f1):8443/v1/health"; then
        echo; break
    fi
    sleep 1
done
systemctl --no-pager --lines=8 status anotau
