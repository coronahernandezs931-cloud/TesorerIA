"""
TesorerIA — Setup de la red de prueba en Stellar Testnet.

Crea (una sola vez) todo el "mundo" del demo:
  - Emisor de los activos de prueba MXNe (peso digital) y USDC.
  - Dueño (admin del contrato), Agente IA (operator), Tesorería de la empresa.
  - Market maker que da liquidez MXNe<->USDC en el DEX nativo (para Path Payments).
  - Proveedores nacionales (MXNe), internacionales (USDC) y empleados (nómina).

Salida:
  state/wallets.secret.json  -> llaves secretas (NO se sube a git)
  state/public_state.json    -> catálogo público (direcciones, proveedores, facturas)

Uso:  .venv/Scripts/python.exe herramientas/setup_stellar.py
"""
import json
import pathlib
import sys
import time
import uuid

import requests
from stellar_sdk import Asset, Keypair, Network, Server, TransactionBuilder

HORIZON = "https://horizon-testnet.stellar.org"
PASS = Network.TESTNET_NETWORK_PASSPHRASE
server = Server(HORIZON)
STATE = pathlib.Path(__file__).resolve().parent.parent / "state"
STATE.mkdir(exist_ok=True)
SECRET_FILE = STATE / "wallets.secret.json"
PUBLIC_FILE = STATE / "public_state.json"

COMPANY = {
    "name": "Grupo Industrial Tlahuica",
    "short": "Tlahuica",
    "city": "Cuernavaca, Morelos",
    "rfc": "GIT190514MX7",
}

SUPPLIERS = [
    # key, nombre, ciudad, país, moneda, tope por tx (MXNe) / None si internacional, whitelisted
    ("logistica", "Logística del Sur", "Cuernavaca", "MX", "MXNe", 100_000, True),
    ("empaques", "Empaques Jiutepec", "Jiutepec", "MX", "MXNe", 40_000, True),
    ("solar", "Energía Solar Morelos", "Temixco", "MX", "MXNe", 150_000, True),
    ("brazos", "Brazos Bearing Co.", "Houston, TX", "US", "USDC", None, True),
    ("textiles", "Textiles Medellín", "Medellín", "CO", "USDC", None, True),
    ("fantasma", "Consultores Fantasma", "Desconocido", "MX", "MXNe", None, False),
]

EMPLOYEES = [
    ("ana", "Ana Torres", "Jefa de Producción", 16_400),
    ("luis", "Luis Ramírez", "Operador CNC", 9_800),
    ("maria", "María Domínguez", "Control de Calidad", 11_200),
    ("jorge", "Jorge Salgado", "Almacén y Logística", 8_900),
]

# RFC de cada proveedor (ficticios, con formato válido). Extranjeros usan el RFC genérico del SAT.
RFC = {"logistica": "LSU150312KJ4", "empaques": "EJI180905TQ1", "solar": "ESM200117HB8",
       "brazos": "XEXX010101000", "textiles": "XEXX010101000", "fantasma": "CFA190221R56"}
# Lista 69-B simulada (EFOS: empresas que facturan operaciones simuladas)
LISTA_69B = ["CFA190221R56"]

INVOICES = [
    ("F-2041", "logistica", 18_500, "MXNe", "Fletes Cuernavaca–CDMX, semana 46"),
    ("F-2042", "empaques", 7_240, "MXNe", "Cajas corrugadas, lote 118"),
    ("F-2043", "solar", 86_000, "MXNe", "Mantenimiento de paneles, planta 2"),
    ("F-2044", "brazos", 3_200, "USDC", "Rodamientos industriales SKF"),
    ("F-2045", "textiles", 1_150, "USDC", "Tela técnica antiflama"),
]


def friendbot(pk: str):
    for i in range(4):
        r = requests.get("https://friendbot.stellar.org", params={"addr": pk}, timeout=60)
        if r.status_code == 200 or "createAccountAlreadyExist" in r.text:
            return
        time.sleep(2 + i * 2)
    raise RuntimeError(f"Friendbot falló para {pk}: {r.text[:200]}")


def submit(source_kp: Keypair, ops_fn, signers, memo=None):
    acct = server.load_account(source_kp.public_key)
    tb = TransactionBuilder(acct, PASS, base_fee=1000).set_timeout(120)
    ops_fn(tb)
    if memo:
        tb.add_text_memo(memo)
    tx = tb.build()
    for s in signers:
        tx.sign(s)
    res = server.submit_transaction(tx)
    print(f"   ✓ tx {res['hash'][:12]}…  ({len(tx.transaction.operations)} ops)")
    return res["hash"]


def main():
    if SECRET_FILE.exists() and "--force" not in sys.argv:
        print("Ya existe state/wallets.secret.json. Usa --force para regenerar todo.")
        return

    kp = {k: Keypair.random() for k in ["issuer", "owner", "agent", "treasury", "mm"]}
    fraud_kp = Keypair.random()  # cuenta del estafador que "cambia" la cuenta de Logística del Sur
    sup_kp = {s[0]: Keypair.random() for s in SUPPLIERS}
    emp_kp = {e[0]: Keypair.random() for e in EMPLOYEES}

    print("1) Fondeando cuentas base con Friendbot…")
    for name, k in kp.items():
        friendbot(k.public_key)
        print(f"   ✓ {name:9s} {k.public_key}")

    MXNe = Asset("MXNe", kp["issuer"].public_key)
    USDC = Asset("USDC", kp["issuer"].public_key)
    treasury = kp["treasury"]

    print("2) Creando cuentas de proveedores y empleados (1 transacción)…")
    created = [k for key, k in sup_kp.items() if key != "fantasma"] + list(emp_kp.values())

    def create_ops(tb):
        for k in created:
            tb.append_create_account_op(destination=k.public_key, starting_balance="6")

    submit(treasury, create_ops, [treasury], memo="TesorerIA onboarding")

    print("3) Trustlines (líneas de confianza) para MXNe/USDC…")
    sup_currency = {s[0]: s[4] for s in SUPPLIERS}

    def trust_ops(tb):
        for k in (treasury, kp["mm"]):
            tb.append_change_trust_op(asset=MXNe, source=k.public_key)
            tb.append_change_trust_op(asset=USDC, source=k.public_key)
        for key, k in sup_kp.items():
            if key == "fantasma":
                continue
            tb.append_change_trust_op(asset=MXNe if sup_currency[key] == "MXNe" else USDC, source=k.public_key)
        for k in emp_kp.values():
            tb.append_change_trust_op(asset=MXNe, source=k.public_key)

    signers = [treasury, kp["mm"]] + [k for key, k in sup_kp.items() if key != "fantasma"] + list(emp_kp.values())
    submit(treasury, trust_ops, signers)

    print("4) Emitiendo MXNe y USDC de prueba…")

    def issue_ops(tb):
        tb.append_payment_op(treasury.public_key, MXNe, "2500000")
        tb.append_payment_op(treasury.public_key, USDC, "25000")
        tb.append_payment_op(kp["mm"].public_key, MXNe, "20000000")
        tb.append_payment_op(kp["mm"].public_key, USDC, "1000000")

    submit(kp["issuer"], issue_ops, [kp["issuer"]])

    print("5) Market maker publica libro de órdenes MXNe/USDC en el DEX…")

    def offer_ops(tb):
        # Vende USDC a cambio de MXNe (precio = MXNe por 1 USDC)
        for amount, price in [("50000", "18.42"), ("150000", "18.47"), ("300000", "18.55")]:
            tb.append_manage_sell_offer_op(selling=USDC, buying=MXNe, amount=amount, price=price)
        # Vende MXNe a cambio de USDC (precio = USDC por 1 MXNe)
        for amount, price in [("900000", "0.0546"), ("2000000", "0.0545")]:
            tb.append_manage_sell_offer_op(selling=MXNe, buying=USDC, amount=amount, price=price)

    submit(kp["mm"], offer_ops, [kp["mm"]])

    secrets = {
        **{k: v.secret for k, v in kp.items()},
        "suppliers": {k: v.secret for k, v in sup_kp.items()},
        "employees": {k: v.secret for k, v in emp_kp.items()},
    }
    SECRET_FILE.write_text(json.dumps(secrets, indent=2))

    public = {
        "network": "testnet",
        "horizon": HORIZON,
        "soroban_rpc": "https://soroban-testnet.stellar.org",
        "company": COMPANY,
        "accounts": {k: v.public_key for k, v in kp.items()},
        "assets": {
            "MXNe": {"code": "MXNe", "issuer": kp["issuer"].public_key},
            "USDC": {"code": "USDC", "issuer": kp["issuer"].public_key},
        },
        "suppliers": [
            {
                "key": key, "name": name, "city": city, "country": country, "currency": cur,
                "max_per_tx": mx, "whitelisted": wl,
                "alias": f"{key}*tesoreria.mx", "rfc": RFC[key],
                "address": sup_kp[key].public_key,
            }
            for key, name, city, country, cur, mx, wl in SUPPLIERS
        ],
        "employees": [
            {"key": key, "name": name, "role": role, "salary": sal, "address": emp_kp[key].public_key}
            for key, name, role, sal in EMPLOYEES
        ],
        "invoices": [
            {"id": i, "supplier": s, "amount": a, "currency": c, "concept": d, "uuid": str(uuid.uuid4()).upper()}
            for i, s, a, c, d in INVOICES
        ],
        "policy": {"daily_limit": 150_000, "approval_threshold": 50_000},
        "lista_69b": LISTA_69B,
        "change_requests": [{
            "id": "CR-1", "supplier": "logistica", "new_address": fraud_kp.public_key,
            "from_email": "pagos.logisticadelsur@gmail.com", "official_email": "pagos@logisticadelsur.mx",
            "subject": "Actualización de datos bancarios", "account_age_days": 2,
        }],
        "contracts": {},
    }
    PUBLIC_FILE.write_text(json.dumps(public, indent=2, ensure_ascii=False))
    print(f"\nListo. Estado guardado en {PUBLIC_FILE.name} y {SECRET_FILE.name}")


if __name__ == "__main__":
    main()
