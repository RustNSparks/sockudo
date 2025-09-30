#!/bin/bash

echo "========================================="
echo "Sockudo Unix Socket Test Environment"
echo "========================================="
echo ""

# Generate self-signed certificate for Nginx HTTPS
if [ ! -f /etc/ssl/certs/nginx.crt ]; then
    echo "[INIT] Generating self-signed certificate for Nginx..."
    openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
        -keyout /etc/ssl/private/nginx.key \
        -out /etc/ssl/certs/nginx.crt \
        -subj "/C=US/ST=State/L=City/O=Sockudo/CN=localhost" 2>/dev/null
    chmod 644 /etc/ssl/certs/nginx.crt
    chmod 600 /etc/ssl/private/nginx.key
    echo "[INIT] SSL certificate generated successfully"
else
    echo "[INIT] SSL certificate already exists"
fi

# Ensure socket directory exists with proper permissions
echo "[INIT] Setting up Unix socket directory..."
mkdir -p /var/run/sockudo
chmod 755 /var/run/sockudo

# Clean up any existing socket file
rm -f /var/run/sockudo/sockudo.sock

# Start a background process to fix socket permissions after creation
(
    while true; do
        if [ -S /var/run/sockudo/sockudo.sock ]; then
            chmod 777 /var/run/sockudo/sockudo.sock
            echo "[INIT] Unix socket created and permissions set to 777"
            break
        fi
        sleep 0.1
    done
) &

echo "[INIT] Starting services..."
echo "========================================="
echo ""

# Start supervisor to manage both processes
exec /usr/bin/supervisord -c /etc/supervisor/conf.d/supervisord.conf