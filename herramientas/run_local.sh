#!/bin/bash
# Servidor local para probar la interfaz: http://localhost:8787
cd "$(dirname "$0")/.." && PYTHONUTF8=1 .venv/Scripts/python.exe -m uvicorn server:api --port 8787
