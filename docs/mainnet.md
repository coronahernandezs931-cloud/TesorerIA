# TesorerIA en Stellar Public Mainnet

**Pantalla en producción:** [**https://tesoreria.lat/mainnet**](https://tesoreria.lat/mainnet) (enlazada desde la barra lateral del tablero principal en [`web/index.html#L271`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/index.html#L271)).

## Contratos Desplegados en Stellar Mainnet (`public`)
1. **`TreasuryMainnet` (Lista blanca, topes y liquidación de facturas CFDI):**
   - Contrato: [`CDPDGQLXFSKWFAPKE5OL6DUWRYLEZ3SWL5ENMXEYCZ47H7Z6IHBSHRS7`](https://stellar.expert/explorer/public/contract/CDPDGQLXFSKWFAPKE5OL6DUWRYLEZ3SWL5ENMXEYCZ47H7Z6IHBSHRS7)
   - Código fuente Rust: [`contracts/treasury-mainnet/src/lib.rs`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/contracts/treasury-mainnet/src/lib.rs)
2. **`TreasuryMainnetOps` (Retención fiscal IVA/ISR, nómina en lote y barrido de reserva):**
   - Contrato: [`CDBUJWHQ34RJS2WLK3RUOHIPQHHT5BCMIE3JIQZPSSDBXMFJHABQDLWC`](https://stellar.expert/explorer/public/contract/CDBUJWHQ34RJS2WLK3RUOHIPQHHT5BCMIE3JIQZPSSDBXMFJHABQDLWC)
   - Código fuente Rust: [`contracts/treasury-mainnet-ops/src/lib.rs`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/contracts/treasury-mainnet-ops/src/lib.rs)

---

## Las 5 Funciones en Mainnet y Cómo las Usa el Frontend (`tesoreria.lat/mainnet`)

### 1. `set_supplier(supplier, max_tx)` — Alta y Límite en Lista Blanca
- **Qué hace:** Autoriza a un proveedor en la lista blanca de la bóveda Mainnet y fija su monto máximo permitido por transacción.
- **Contrato en Mainnet:** [`contracts/treasury-mainnet/src/lib.rs#L21`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/contracts/treasury-mainnet/src/lib.rs#L21) (`CDPDGQLXFSKWFAPKE5OL6DUWRYLEZ3SWL5ENMXEYCZ47H7Z6IHBSHRS7`)
- **Componente / Pantalla en Frontend:** Pantalla `/mainnet`, tarjeta `#card-set-supplier` en [`web/mainnet.html#L67`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/mainnet.html#L67)
- **Cómo se llama desde el Frontend:** Manejador `#btnFn1` → `invokeMainnet(..., '/api/mainnet/set-supplier', ...)` en [`web/mainnet.html#L203`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/mainnet.html#L203), atendido por `mainnet_set_supplier()` en [`server.py#L1285`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/server.py#L1285).
- **Transacción real ejecutada desde el frontend:** [`2726379d5447141ddb9b9514b63093f1ab86da7d1622f7c9f4dfdcf78526d87b`](https://stellar.expert/explorer/public/tx/2726379d5447141ddb9b9514b63093f1ab86da7d1622f7c9f4dfdcf78526d87b)

### 2. `pay_invoice(supplier, amount, folio)` — Liquidación de Factura con Candado Anti-Doble Pago CFDI
- **Qué hace:** Verifica que el proveedor esté activo en lista blanca, que el monto no exceda el tope individual ni el límite diario, y que el folio fiscal (`folio`) jamás haya sido pagado antes de transferir XLM reales vía Stellar Asset Contract (SAC).
- **Contrato en Mainnet:** [`contracts/treasury-mainnet/src/lib.rs#L30`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/contracts/treasury-mainnet/src/lib.rs#L30) (`CDPDGQLXFSKWFAPKE5OL6DUWRYLEZ3SWL5ENMXEYCZ47H7Z6IHBSHRS7`)
- **Componente / Pantalla en Frontend:** Pantalla `/mainnet`, tarjeta `#card-pay-invoice` en [`web/mainnet.html#L79`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/mainnet.html#L79)
- **Cómo se llama desde el Frontend:** Manejador `#btnFn2` → `invokeMainnet(..., '/api/mainnet/pay', ...)` en [`web/mainnet.html#L207`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/mainnet.html#L207), atendido por `mainnet_pay()` en [`server.py#L1299`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/server.py#L1299).
- **Transacción real ejecutada desde el frontend:** [`cedba955671419423a1db03d797ec30fb3c716ed96b651707f0d9442c914797f`](https://stellar.expert/explorer/public/tx/cedba955671419423a1db03d797ec30fb3c716ed96b651707f0d9442c914797f)

### 3. `pay_with_tax(supplier, tax_vault, gross, tax_bps, folio)` — Pago con Retención Fiscal Automática SAT (IVA/ISR)
- **Qué hace:** Calcula en cadena la retención fiscal en puntos base (`tax_bps`), transfiere el importe neto al proveedor autorizado y segrega atómicamente el impuesto retenido hacia la bóveda fiscal (`tax_vault`), bloqueando el folio contra reutilización.
- **Contrato en Mainnet:** [`contracts/treasury-mainnet-ops/src/lib.rs#L19`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/contracts/treasury-mainnet-ops/src/lib.rs#L19) (`CDBUJWHQ34RJS2WLK3RUOHIPQHHT5BCMIE3JIQZPSSDBXMFJHABQDLWC`)
- **Componente / Pantalla en Frontend:** Pantalla `/mainnet`, tarjeta `#card-pay-with-tax` en [`web/mainnet.html#L91`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/mainnet.html#L91)
- **Cómo se llama desde el Frontend:** Manejador `#btnFn3` → `invokeMainnet(..., '/api/mainnet/pay-tax', ...)` en [`web/mainnet.html#L211`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/mainnet.html#L211), atendido por `mainnet_pay_tax()` en [`server.py#L1313`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/server.py#L1313).
- **Transacción real ejecutada desde el frontend:** [`3af7772fe29de34c72f64b5f5c665a3341888fe3e2a939bb4152c5a241cc74c7`](https://stellar.expert/explorer/public/tx/3af7772fe29de34c72f64b5f5c665a3341888fe3e2a939bb4152c5a241cc74c7)

### 4. `batch_payroll(w1, w2, amount_each, period)` — Dispersión Atómica de Nómina en Lote
- **Qué hace:** Liquida en una sola transacción atómica de Soroban la nómina quincenal a múltiples destinatarios sobre Stellar Mainnet y marca el periodo (`period`) como liquidado para impedir pagos duplicados de quincena.
- **Contrato en Mainnet:** [`contracts/treasury-mainnet-ops/src/lib.rs#L43`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/contracts/treasury-mainnet-ops/src/lib.rs#L43) (`CDBUJWHQ34RJS2WLK3RUOHIPQHHT5BCMIE3JIQZPSSDBXMFJHABQDLWC`)
- **Componente / Pantalla en Frontend:** Pantalla `/mainnet`, tarjeta `#card-batch-payroll` en [`web/mainnet.html#L103`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/mainnet.html#L103)
- **Cómo se llama desde el Frontend:** Manejador `#btnFn4` → `invokeMainnet(..., '/api/mainnet/payroll', ...)` en [`web/mainnet.html#L215`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/mainnet.html#L215), atendido por `mainnet_payroll()` en [`server.py#L1329`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/server.py#L1329).
- **Transacción real ejecutada desde el frontend:** [`56d63fbbb4ad7bef7510ba4cd8c1ec670b30294d7ed14298d858d41379a4c5de`](https://stellar.expert/explorer/public/tx/56d63fbbb4ad7bef7510ba4cd8c1ec670b30294d7ed14298d858d41379a4c5de)

### 5. `sweep_reserve(dest, amount)` — Barrido Seguro de Reserva Excedente al Director
- **Qué hace:** Permite a la administración retirar liquidez excedente desde la bóveda operativa del contrato hacia la cuenta maestra de tesorería (`dest`) y devuelve el saldo remanente en bóveda.
- **Contrato en Mainnet:** [`contracts/treasury-mainnet-ops/src/lib.rs#L65`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/contracts/treasury-mainnet-ops/src/lib.rs#L65) (`CDBUJWHQ34RJS2WLK3RUOHIPQHHT5BCMIE3JIQZPSSDBXMFJHABQDLWC`)
- **Componente / Pantalla en Frontend:** Pantalla `/mainnet`, tarjeta `#card-sweep-reserve` en [`web/mainnet.html#L115`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/mainnet.html#L115)
- **Cómo se llama desde el Frontend:** Manejador `#btnFn5` → `invokeMainnet(..., '/api/mainnet/sweep', ...)` en [`web/mainnet.html#L219`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/web/mainnet.html#L219), atendido por `mainnet_sweep()` en [`server.py#L1344`](https://github.com/coronahernandezs931-cloud/TesorerIA/blob/main/server.py#L1344).
- **Transacción real ejecutada desde el frontend:** [`49a127b2a8fb7883edf8b844c307ac5cbc142ae76f38c9b3880d5e893d228898`](https://stellar.expert/explorer/public/tx/49a127b2a8fb7883edf8b844c307ac5cbc142ae76f38c9b3880d5e893d228898)
