#!/bin/bash
# Despliega TreasuryGuard + SAC de MXNe en testnet, da de alta proveedores y fondea la bóveda.
set -e
export PATH=$HOME/.local/bin:$PATH
ST=/mnt/c/Users/coron/Downloads/ProyGoya/tesoreria/state
j() { python3 -c "import json,sys;d=json.load(open('$1'));print(eval(sys.argv[1]))" "$2"; }
OWNER_S=$(j $ST/wallets.secret.json "d['owner']"); TREAS_S=$(j $ST/wallets.secret.json "d['treasury']")
OWNER=$(j $ST/public_state.json "d['accounts']['owner']"); AGENT=$(j $ST/public_state.json "d['accounts']['agent']")
TREAS=$(j $ST/public_state.json "d['accounts']['treasury']"); ISSUER=$(j $ST/public_state.json "d['assets']['MXNe']['issuer']")
N="--network testnet"
echo "== SAC de MXNe"
SAC=$(stellar contract asset deploy --asset MXNe:$ISSUER --source-account $OWNER_S $N 2>/dev/null || stellar contract id asset --asset MXNe:$ISSUER $N)
echo "SAC=$SAC"
echo "== Deploy TreasuryGuard (limite diario 150k, umbral 50k MXNe)"
GUARD=$(stellar contract deploy --wasm $ST/treasury_guard.wasm --source-account $OWNER_S $N -- \
  --admin $OWNER --operator $AGENT --token $SAC --daily_limit 1500000000000 --approval_threshold 500000000000)
echo "GUARD=$GUARD"
python3 - "$SAC" "$GUARD" <<PY
import json,sys; p='$ST/public_state.json'; d=json.load(open(p))
d['contracts']={'mxnb_sac':sys.argv[1],'treasury_guard':sys.argv[2]}
json.dump(d,open(p,'w'),indent=2,ensure_ascii=False)
PY
echo "== Alta de proveedores en lista blanca"
python3 -c "
import json;d=json.load(open('$ST/public_state.json'))
for s in d['suppliers']:
  if s['currency']=='MXNe' and s['whitelisted']: print(s['address'], s['max_per_tx'], s['name'].replace(' ','_'))
" | while read ADDR MAX NAME; do
  stellar contract invoke --id $GUARD --source-account $OWNER_S $N --send=yes -- set_supplier --supplier $ADDR --name "${NAME//_/ }" --max_per_tx ${MAX}0000000 >/dev/null && echo "  ✓ $NAME"
done
echo "== Fondeo de la bóveda: 1,000,000 MXNe desde tesorería"
stellar contract invoke --id $SAC --source-account $TREAS_S $N --send=yes -- transfer --from $TREAS --to $GUARD --amount 10000000000000
echo "== Saldo de la bóveda:"; stellar contract invoke --id $GUARD --source-account $OWNER_S $N -- balance
