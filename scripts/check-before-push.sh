#!/usr/bin/env bash
# Push öncesi sızıntı kontrolü.
#
# Kullanım:
#   ./scripts/check-before-push.sh            # bu repoyu tara
#   ./scripts/check-before-push.sh /some/dir  # başka bir klasörü tara
#
# Çıkış kodu 0 = temiz, 1 = bulgu var (push etmeyin).

set -uo pipefail

TARGET="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$TARGET" || { echo "Hata: $TARGET bulunamadı"; exit 2; }

# Taranmayacaklar
EXCLUDES=(
  --exclude-dir=.git
  --exclude-dir=target
  --exclude-dir=node_modules
  --exclude-dir=.venv
  --exclude='*.png' --exclude='*.jpg' --exclude='*.pdf'
  --exclude='Cargo.lock'
  --exclude='check-before-push.sh'
)

# Her kural: "etiket|regex"
RULES=(
  'AWS access key ID|AKIA[0-9A-Z]{16}'
  'AWS ARN|arn:aws:[a-z0-9-]+:[a-z0-9-]*:[0-9]{12}'
  '12 haneli hesap numarası|(^|[^0-9])[0-9]{12}([^0-9]|$)'
  'Özel anahtar bloğu|-----BEGIN [A-Z ]*PRIVATE KEY-----'
  'iyzico/ödeme anahtarı|(api[_-]?key|secret[_-]?key|private[_-]?key)[[:space:]]*[:=][[:space:]]*["'\'']?[A-Za-z0-9._-]{12,}'
  'Parola/token ataması|(password|passwd|token)[[:space:]]*[:=][[:space:]]*["'\'']?[A-Za-z0-9._-]{8,}'
  'AWS hesap ID (bilinen)|356650046692'
  'API Gateway route/integration ID|"(RouteId|Target)"[[:space:]]*:[[:space:]]*"[a-z0-9]{5,}"'
)

FOUND=0

for rule in "${RULES[@]}"; do
  label="${rule%%|*}"
  pattern="${rule#*|}"

  hits=$(grep -rInE "${EXCLUDES[@]}" -- "$pattern" . 2>/dev/null \
         | grep -vE '^\./docs/what-is-not-here\.md' \
         | grep -vE '^\./scripts/check-before-push\.sh')

  if [ -n "$hits" ]; then
    echo ""
    echo "✗ BULGU: $label"
    echo "$hits" | head -12 | sed 's/^/    /'
    total=$(echo "$hits" | wc -l)
    if [ "$total" -gt 12 ]; then echo "    ... ve $((total - 12)) satır daha"; fi
    FOUND=1
  fi
done

echo ""
if [ "$FOUND" -eq 0 ]; then
  echo "✓ Temiz — $(pwd) içinde bilinen sızıntı deseni bulunamadı."
  exit 0
else
  echo "✗ Push ETMEYİN. Yukarıdaki satırları silin veya genericleştirin, sonra tekrar çalıştırın."
  exit 1
fi
