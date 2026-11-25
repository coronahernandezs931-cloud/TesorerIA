"""Genera facturas CFDI 4.0 de ejemplo para el demo (ficticias, sin sello real)."""
import json, pathlib, uuid
ROOT = pathlib.Path(__file__).resolve().parent.parent
PUB = json.loads((ROOT / "state/public_state.json").read_text("utf-8"))
RFC_RECEPTOR = PUB["company"]["rfc"]
OUT = ROOT / "demo_cfdi"

def cfdi(serie, folio, emisor_rfc, emisor_nombre, concepto, subtotal, moneda="MXN"):
    iva = round(subtotal * 0.16, 2); total = round(subtotal + iva, 2)
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<cfdi:Comprobante xmlns:cfdi="http://www.sat.gob.mx/cfd/4" xmlns:tfd="http://www.sat.gob.mx/TimbreFiscalDigital"
  Version="4.0" Serie="{serie}" Folio="{folio}" Fecha="2026-11-20T10:15:00" SubTotal="{subtotal:.2f}" Moneda="{moneda}"
  Total="{total:.2f}" TipoDeComprobante="I" MetodoPago="PPD" FormaPago="99" LugarExpedicion="62550" Exportacion="01">
  <cfdi:Emisor Rfc="{emisor_rfc}" Nombre="{emisor_nombre}" RegimenFiscal="601"/>
  <cfdi:Receptor Rfc="{RFC_RECEPTOR}" Nombre="GRUPO INDUSTRIAL TLAHUICA" DomicilioFiscalReceptor="62000" RegimenFiscalReceptor="601" UsoCFDI="G03"/>
  <cfdi:Conceptos>
    <cfdi:Concepto ClaveProdServ="24112700" Cantidad="1" ClaveUnidad="E48" Descripcion="{concepto}" ValorUnitario="{subtotal:.2f}" Importe="{subtotal:.2f}" ObjetoImp="02"/>
  </cfdi:Conceptos>
  <cfdi:Complemento>
    <tfd:TimbreFiscalDigital Version="1.1" UUID="{str(uuid.uuid4()).upper()}" FechaTimbrado="2026-11-20T10:16:02" RfcProvCertif="SAT970701NN3" SelloCFD="DEMO" NoCertificadoSAT="00001000000000000000" SelloSAT="DEMO"/>
  </cfdi:Complemento>
</cfdi:Comprobante>
'''

ok = cfdi("EJ", "3391", "EJI180905TQ1", "EMPAQUES JIUTEPEC", "Tarimas de madera tratada, lote 42", 10_758.62)
(OUT / "1_empaques_jiutepec_EJ-3391.xml").write_text(ok, "utf-8")
(OUT / "2_empaques_jiutepec_EJ-3391_REENVIADA.xml").write_text(ok, "utf-8")  # mismo UUID: duplicado
(OUT / "3_consultores_fantasma_CF-0007.xml").write_text(
    cfdi("CF", "0007", "CFA190221R56", "CONSULTORES FANTASMA", "Asesoría estratégica integral", 81_896.55), "utf-8")
print("listo:", [p.name for p in OUT.iterdir()])
