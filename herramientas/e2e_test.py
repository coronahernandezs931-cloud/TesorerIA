"""Prueba de punta a punta contra Stellar Testnet (sin voz). Uso: .venv/Scripts/python.exe -X utf8 herramientas/e2e_test.py"""
import json
import os
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

os.environ.setdefault("DASH_KEY", "test-dash")
os.environ.setdefault("TOOLS_KEY", "test-tools")
from fastapi.testclient import TestClient  # noqa: E402

import server  # noqa: E402

c = TestClient(server.api)
D = {"x-dash-key": os.environ["DASH_KEY"]}
GUEST_CALL = {"call": {"call_type": "web_call", "retell_llm_dynamic_variables": {"rol": "invitado"}}}
ok = True


def call(path, role="director", **args):
    body = {"args": args, **(GUEST_CALL if role == "invitado" else {})}
    headers = D if role == "director" else {"x-tesa-key": os.environ["TOOLS_KEY"]}
    j = c.post(path, json=body, headers=headers).json()
    print(f"\n>> [{role}] {path} {args}\n   {j.get('mensaje') or json.dumps(j, ensure_ascii=False)[:200]}")
    return j


def expect(cond, label):
    global ok
    print(("   ✅ " if cond else "   ❌ ") + label)
    ok &= bool(cond)


expect(c.post("/tools/resumen_tesoreria", json={}).status_code == 401, "sin llave: herramientas cerradas")
expect(c.get("/api/state").json()["dash"] is False, "sin llave: tablero en solo lectura")
expect(c.post("/api/demo/reset", json={}, headers=D).json().get("ok"), "reinicio con llave")

call("/tools/resumen_tesoreria")
r = call("/tools/pagar_factura", factura="F-2041"); expect(r.get("ok") and r.get("hash"), "pago nacional por contrato")
r = call("/tools/pagar_factura", factura="F-2041"); expect(r.get("codigo") == "DuplicateInvoice", "el contrato rechaza pagar dos veces")
r = call("/tools/pagar_factura", role="invitado", factura="F-2042"); expect(r.get("solicitud_registrada"), "invitado no puede pagar")
r = call("/tools/pagar_a_cuenta_nueva", role="invitado", beneficiario="Juez Pérez", monto=50000); expect(r.get("bloqueado"), "invitado: 'págame a mí' bloqueado on-chain")
r = call("/tools/pagar_a_cuenta_nueva", proveedor="Logística del Sur"); expect(r.get("posible_fraude"), "cambio de cuenta = alerta de fraude")
r = call("/tools/pagar_factura", proveedor="brazos"); expect(r.get("ok"), "pago internacional path payment")
r = call("/tools/pagar_factura", factura="F-2043"); expect(r.get("requiere_aprobacion"), "pago grande retenido")

st = c.get("/api/state", headers=D).json()
aps = {a["type"]: a for a in st["approvals"]}
expect(aps.get("payment", {}).get("token"), "con llave: el tablero ve la liga de aprobación")
expect(c.get("/api/state").json()["approvals"][0]["token"] is None, "sin llave: la liga no se expone")
r = c.post("/api/approve", json={"id": aps["payment"]["id"], "t": aps["payment"]["token"]}).json(); expect(r.get("ok"), "director aprueba")
r = c.post("/api/approve", json={"id": aps["supplier_change"]["id"], "t": aps["supplier_change"]["token"], "decision": "reject"}).json()
expect(r.get("ok"), "director confirma el fraude")

files = sorted((ROOT / "demo_cfdi").glob("*.xml"))
for f in files:
    j = c.post("/api/cfdi", content=f.read_bytes(), headers=D).json()
    print(f"\n>> CFDI {f.name}: {j.get('mensaje')}")
expect(any(v.get("cfdi") and v["status"] == "pendiente" for v in c.get("/api/state", headers=D).json()["invoices"]), "CFDI válido registrado")
r = call("/tools/pagar_factura", factura="la última"); expect(r.get("ok") or r.get("bloqueado") is not None, "pagar la factura recién subida")
r = call("/tools/me_alcanza")
r = call("/tools/pagar_nomina"); expect(r.get("ok"), "nómina en lote")
print("\nchain events:", len(c.get("/api/chain").json()["events"]))
print("\nRESULTADO:", "TODO BIEN ✅" if ok else "HAY FALLAS ❌")
