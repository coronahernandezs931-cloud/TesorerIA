"""
Despliegue de TesorerIA en Modal.
  modal deploy tesoreria/modal_app.py
URL pública: https://<usuario>--tesoreria.modal.run
"""
import modal

app = modal.App("tesoreria-ia")

image = (
    modal.Image.debian_slim(python_version="3.11")
    .pip_install("stellar-sdk==17.*", "fastapi[standard]", "requests")
    .env({"WEB_DIR": "/root/web", "PUBLIC_STATE": "/root/public_state.json", "USE_MODAL_DICT": "1"})
    .add_local_file("tesoreria/state/public_state.json", "/root/public_state.json")
    .add_local_dir("tesoreria/web", "/root/web")
    .add_local_file("tesoreria/server.py", "/root/server.py")
)


@app.function(
    image=image,
    secrets=[modal.Secret.from_name("tesoreria-stellar"), modal.Secret.from_name("tesoreria-retell"), modal.Secret.from_name("tesoreria-mainnet")],  # la segunda gana: cuenta de Retell con saldo
    min_containers=1,  # sin arranque en frío durante el demo (latencia de voz)
    max_containers=1,  # un solo contenedor = secuencia de cuentas Stellar consistente
    scaledown_window=900,
)
@modal.concurrent(max_inputs=40)
@modal.asgi_app(label="tesoreria")
def web():
    import sys

    sys.path.insert(0, "/root")
    from server import api

    return api
