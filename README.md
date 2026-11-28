# TesorerIA

**Demo pública en vivo:** [**https://tesoreria.lat**](https://tesoreria.lat) · **Stellar Mainnet en vivo:** [**tesoreria.lat/mainnet**](https://tesoreria.lat/mainnet) ([`docs/mainnet.md`](docs/mainnet.md)) · Reto del Jurado: [`tesoreria.lat/reto`](https://tesoreria.lat/reto) · SEP-1: [`tesoreria.lat/.well-known/stellar.toml`](https://tesoreria.lat/.well-known/stellar.toml)

El candado antifraude de tus pagos: **la voz sugiere, el contrato manda, la factura respalda.**
Tesorera digital por voz para PyMEs sobre **Stellar + Soroban** con validación de facturas SAT (CFDI), Path Payments atómicos (`MXNe → USDC`) y nómina en lote.

## Qué lanzamos el 27 de noviembre (Ship It: Builder Challenge)

Al cierre del **27 de noviembre de 2026** lanzamos **TesorerIA v1 en producción sobre Stellar**:

1. **Contrato inteligente `TreasuryGuard` en Stellar Mainnet** (hoy activo en Testnet v2: [`CBTYXTKM3MQ47G4GG6U6YCDU5WHVPYLCSRC4NZY6SWWSNRVUJF4MDAF2`](https://stellar.expert/explorer/testnet/contract/CBTYXTKM3MQ47G4GG6U6YCDU5WHVPYLCSRC4NZY6SWWSNRVUJF4MDAF2) y **v4 financiero con 24 tests unitarios y 0 warnings**: [`CCFOB4PWZMTZZMWVZ5RP5YGJPKP57NF7W4OJHBXAPJ3INVIGWXY7WGB2`](https://stellar.expert/explorer/testnet/contract/CCFOB4PWZMTZZMWVZ5RP5YGJPKP57NF7W4OJHBXAPJ3INVIGWXY7WGB2)) con reglas on-chain: lista blanca de proveedores, tope por transacción, límite diario global, umbral de firma humana (>50,000), candado anti-doble pago por folio fiscal (UUID del SAT), *circuit breaker* (`pause_vault`/`unpause_vault`), pagos atómicos en lote (`batch_pay`), barrido con protección de solvencia (`withdraw_reserve`), pagos programados con *timelock* (`schedule_payment`/`execute_scheduled`), expiración de retenidos (`expire_payment`), retención fiscal automática ISR/IVA (`pay_with_withholding`), parcialidades en escrow con anticipo (`open_installment_plan`/`pay_installment`), aplicación de notas de crédito CFDI de egreso (`settle_with_credit_note`), pronto pago con barrido de ahorro (`pay_early_discounted`) y dispersión compartida por centro de costos (`split_cost_center_pay`).
2. **Aplicación web pública en vivo** ([`tesoreria.lat`](https://tesoreria.lat)): tablero de tesorería con verificación directa de eventos Soroban RPC, soporte de memo personalizado en transacciones, validación CFDI/69-B, aprobación móvil por QR y **Reto del Jurado (`/reto`)**.
3. **Agente de voz Tesa (Retell AI)** + **3 rieles nativos de Stellar** (Soroban, Path Payments DEX y nómina en lote) + estándares **SEP-1** (`/.well-known/stellar.toml`) y **SEP-2** (`/federation`).
4. **Video Demo Oficial (YouTube):** [**Ver Demo de TesorerIA en YouTube**](https://www.youtube.com/watch?v=trMrPiDqwtA)
5. **Contrato `TreasuryStreams` v1 en Stellar Testnet** ([`CCMAXTSQZM2ECPI5V2CDUL6TS26FHDP76DVQTT6PAEJG2IB2RM4DGWRU`](https://stellar.expert/explorer/testnet/contract/CCMAXTSQZM2ECPI5V2CDUL6TS26FHDP76DVQTT6PAEJG2IB2RM4DGWRU)): motor complementario con 25 funciones on-chain para nómina continua, aguinaldo/finiquito LFT, fianzas con *slashing*, factoraje inverso, provisiones IVA/ISR/IMSS/PTU, cartas de crédito y coberturas FX.
6. **Contrato `TreasuryCredit` v1 en Stellar Testnet** ([`CA3KEIPQVDIKYDP4CERW5U7S2BXVC3CAF3BFV22UTBTLLHCG27BL5G6V`](https://stellar.expert/explorer/testnet/contract/CA3KEIPQVDIKYDP4CERW5U7S2BXVC3CAF3BFV22UTBTLLHCG27BL5G6V)): 25 funciones on-chain adicionales para líneas de crédito revolvente, tramos CETES, pedimentos aduanales IGI, *earnouts* M&A, dividendos con retención ISR, pólizas de seguro, *Fondo de Ahorro*, tarifa eléctrica CFE y cascada de regalías.
7. **Contrato `TreasurySettlement` v1 en Stellar Testnet** ([`CAET6RS5D34CUGOAVQC54KS3ALPXBBV7OPOJM2YEBNOXQEL6XMSYG7CL`](https://stellar.expert/explorer/testnet/contract/CAET6RS5D34CUGOAVQC54KS3ALPXBBV7OPOJM2YEBNOXQEL6XMSYG7CL)): 25 funciones on-chain para compensación SPEI, arrendamiento financiero, certificados de depósito (warrants), avío agrícola, adquirencia POS, Predial/ISN y caps legales.
8. **Contrato `TreasuryTrade` v1 en Stellar Testnet** ([`CBYHTLRHAACRM4W4OG5HRD3D4MUOAOBSLCDXDVYEQV2C5IKYTEQ4XGMN`](https://stellar.expert/explorer/testnet/contract/CBYHTLRHAACRM4W4OG5HRD3D4MUOAOBSLCDXDVYEQV2C5IKYTEQ4XGMN)): 25 funciones on-chain para factoraje de exportación, maquila IMMEX, rentas industriales FIBRA, energía solar PPA/CEL, licitaciones públicas, ferrocarril intermodal, CONAGUA y prevalidación CAAAREM.

*(Nota de transparencia: en la demo de Testnet los activos MXNe/USDC son de prueba y la consulta al SAT/lista 69-B es simulada.)*

## Dónde está cada cosa

```
tesoreria/
├── contracts/       Contrato en Stellar (treasury-guard/src/lib.rs) y sus pruebas (test.rs)
├── server.py        Servidor: pagos, fraude, facturas y nómina
├── create_agent.py  Configuración de Tesa, la agente de voz
├── modal_app.py     Publicación en Modal
├── web/             Páginas: tablero, aprobación y reto
├── state/           Datos que usan el servidor y el contrato
├── demo_cfdi/       Facturas XML de ejemplo
└── herramientas/    Scripts de apoyo (no hacen falta para usar el sistema)
```

En la computadora de desarrollo hay además `documentos/`, `registros/` y `video/`.

## Para leer el código, en este orden

1. `contracts/treasury-guard/src/lib.rs`: las reglas. Empieza por `request_payment`.
2. `contracts/treasury-guard/src/test.rs`: las pruebas de esas reglas.
3. `server.py`: cómo se le pide un pago al contrato (`pay_domestic`).
4. `create_agent.py`: qué puede pedir la voz.

## Herramientas

| Archivo | Para qué | Cuidado |
|---|---|---|
| `herramientas/run_local.sh` | Servidor local en el puerto 8787 | Usa las cuentas reales de testnet |
| `herramientas/gen_cfdi.py` | Facturas de ejemplo con folio nuevo | Reemplaza las de `demo_cfdi/` |
| `herramientas/e2e_test.py` | Prueba completa | Hace pagos en testnet |
| `herramientas/setup_stellar.py` | Crea las cuentas de prueba | `--force` reemplaza todas las llaves |
| `herramientas/install_wsl_tools.sh` | Instala Rust y Stellar CLI en WSL | Descarga programas |
| `create_agent.py` | Actualiza a Tesa | Modifica el agente en vivo en Retell |

`server.py`, `modal_app.py`, `create_agent.py`, `web/` y `state/` van en esta ubicación:
el despliegue depende de esas rutas.
