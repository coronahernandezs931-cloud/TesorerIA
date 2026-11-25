# TesorerIA

El candado antifraude de tus pagos: **la IA pide, el contrato decide, el director firma.**
Corre en Stellar testnet con activos de prueba. La consulta al SAT y la lista 69-B son simuladas.

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
