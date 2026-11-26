#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    token::StellarAssetClient,
    Address, Env, String,
};

const UNIT: i128 = 10_000_000; // 7 decimales

struct Ctx {
    env: Env,
    guard: TreasuryGuardClient<'static>,
    token: token::Client<'static>,
    supplier: Address,
}

fn setup() -> Ctx {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let token_addr = sac.address();

    let guard_id = env.register(
        TreasuryGuard,
        (admin.clone(), operator.clone(), token_addr.clone(), 150_000 * UNIT, 50_000 * UNIT),
    );
    StellarAssetClient::new(&env, &token_addr).mint(&guard_id, &(1_000_000 * UNIT));

    let guard = TreasuryGuardClient::new(&env, &guard_id);
    let supplier = Address::generate(&env);
    guard.set_supplier(&supplier, &String::from_str(&env, "Logistica del Sur"), &(100_000 * UNIT));
    let token = token::Client::new(&env, &token_addr);
    Ctx { env, guard, token, supplier }
}

#[test]
fn pago_bajo_umbral_se_liquida() {
    let c = setup();
    let p = c.guard.request_payment(&c.supplier, &(18_500 * UNIT), &String::from_str(&c.env, "F-1021"));
    assert_eq!(p.status, Status::Paid);
    assert_eq!(c.token.balance(&c.supplier), 18_500 * UNIT);
    assert_eq!(c.guard.spent_today(), 18_500 * UNIT);
}

#[test]
fn pago_sobre_umbral_requiere_aprobacion() {
    let c = setup();
    let p = c.guard.request_payment(&c.supplier, &(86_000 * UNIT), &String::from_str(&c.env, "F-1023"));
    assert_eq!(p.status, Status::Pending);
    assert_eq!(c.token.balance(&c.supplier), 0);
    let p = c.guard.approve(&p.id);
    assert_eq!(p.status, Status::Paid);
    assert_eq!(c.token.balance(&c.supplier), 86_000 * UNIT);
}

#[test]
fn proveedor_no_autorizado_es_rechazado() {
    let c = setup();
    let intruso = Address::generate(&c.env);
    let r = c.guard.try_request_payment(&intruso, &(1_000 * UNIT), &String::from_str(&c.env, "X"));
    assert_eq!(r, Err(Ok(soroban_sdk::Error::from_contract_error(Error::NotWhitelisted as u32))));
}

#[test]
fn tope_por_transaccion() {
    let c = setup();
    let r = c.guard.try_request_payment(&c.supplier, &(120_000 * UNIT), &String::from_str(&c.env, "X"));
    assert_eq!(r, Err(Ok(soroban_sdk::Error::from_contract_error(Error::ExceedsTxLimit as u32))));
}

#[test]
fn tope_diario() {
    let c = setup();
    for f in ["F1", "F2", "F3"] {
        c.guard.request_payment(&c.supplier, &(45_000 * UNIT), &String::from_str(&c.env, f));
    }
    // 135k gastados; uno más de 45k rebasa 150k
    let r = c.guard.try_request_payment(&c.supplier, &(45_000 * UNIT), &String::from_str(&c.env, "F4"));
    assert_eq!(r, Err(Ok(soroban_sdk::Error::from_contract_error(Error::ExceedsDailyLimit as u32))));
}

#[test]
fn rechazo_del_duenio() {
    let c = setup();
    let p = c.guard.request_payment(&c.supplier, &(60_000 * UNIT), &String::from_str(&c.env, "F"));
    let p = c.guard.reject(&p.id);
    assert_eq!(p.status, Status::Rejected);
    assert_eq!(c.guard.try_approve(&p.id), Err(Ok(soroban_sdk::Error::from_contract_error(Error::NotPending as u32))));
}

#[test]
fn factura_duplicada_es_rechazada() {
    let c = setup();
    let inv = String::from_str(&c.env, "UUID-A1B2");
    c.guard.request_payment(&c.supplier, &(1_000 * UNIT), &inv);
    let r = c.guard.try_request_payment(&c.supplier, &(1_000 * UNIT), &inv);
    assert_eq!(r, Err(Ok(soroban_sdk::Error::from_contract_error(Error::DuplicateInvoice as u32))));
    assert_eq!(c.guard.invoice_payment(&inv), Some(1));
}

#[test]
fn rechazo_libera_la_factura() {
    let c = setup();
    let inv = String::from_str(&c.env, "UUID-X");
    let p = c.guard.request_payment(&c.supplier, &(60_000 * UNIT), &inv);
    c.guard.reject(&p.id);
    let p2 = c.guard.request_payment(&c.supplier, &(60_000 * UNIT), &inv);
    assert_eq!(p2.status, Status::Pending);
}

#[test]
fn aprobar_proveedor_dado_de_baja_falla() {
    let c = setup();
    let p = c.guard.request_payment(&c.supplier, &(60_000 * UNIT), &String::from_str(&c.env, "F9"));
    c.guard.remove_supplier(&c.supplier);
    assert_eq!(c.guard.try_approve(&p.id), Err(Ok(soroban_sdk::Error::from_contract_error(Error::NotWhitelisted as u32))));
}

#[test]
fn reset_day_reinicia_gasto_sin_cambiar_limite() {
    let c = setup();
    c.guard.request_payment(&c.supplier, &(40_000 * UNIT), &String::from_str(&c.env, "R1"));
    c.guard.reset_day();
    assert_eq!(c.guard.spent_today(), 0);
    assert_eq!(c.guard.get_config().daily_limit, 150_000 * UNIT);
}

#[test]
fn pausa_de_emergencia_bloquea_y_reactiva_pagos() {
    let c = setup();
    assert!(!c.guard.is_paused());
    c.guard.pause_vault(&String::from_str(&c.env, "Alerta cambio de cuenta"));
    assert!(c.guard.is_paused());

    let r = c.guard.try_request_payment(&c.supplier, &(5_000 * UNIT), &String::from_str(&c.env, "P-01"));
    assert_eq!(r, Err(Ok(soroban_sdk::Error::from_contract_error(Error::VaultPaused as u32))));

    c.guard.unpause_vault();
    assert!(!c.guard.is_paused());
    let p = c.guard.request_payment(&c.supplier, &(5_000 * UNIT), &String::from_str(&c.env, "P-01"));
    assert_eq!(p.status, Status::Paid);
}

#[test]
fn pago_en_lote_atomico_liquida_multiples_partidas() {
    let c = setup();
    let sup2 = Address::generate(&c.env);
    c.guard.set_supplier(&sup2, &String::from_str(&c.env, "Empaques Jiutepec"), &(40_000 * UNIT));

    let mut lote = Vec::new(&c.env);
    lote.push_back(BatchItem {
        supplier: c.supplier.clone(),
        amount: 12_000 * UNIT,
        invoice: String::from_str(&c.env, "LOTE-01"),
    });
    lote.push_back(BatchItem {
        supplier: sup2.clone(),
        amount: 8_500 * UNIT,
        invoice: String::from_str(&c.env, "LOTE-02"),
    });

    let pagos = c.guard.batch_pay(&lote);
    assert_eq!(pagos.len(), 2);
    assert_eq!(c.token.balance(&c.supplier), 12_000 * UNIT);
    assert_eq!(c.token.balance(&sup2), 8_500 * UNIT);
    assert_eq!(c.guard.spent_today(), 20_500 * UNIT);
}

#[test]
fn retiro_de_reserva_protege_fondos_comprometidos() {
    let c = setup();
    let tesoreria_ext = Address::generate(&c.env);
    // Retiene 90k de un saldo total de 1,000,000 -> quedan 910,000 libres
    c.guard.request_payment(&c.supplier, &(90_000 * UNIT), &String::from_str(&c.env, "RES-01"));

    let fallo = c.guard.try_withdraw_reserve(&tesoreria_ext, &(950_000 * UNIT));
    assert_eq!(fallo, Err(Ok(soroban_sdk::Error::from_contract_error(Error::ReservedFundsLocked as u32))));

    let libre = c.guard.withdraw_reserve(&tesoreria_ext, &(100_000 * UNIT));
    assert_eq!(libre, 810_000 * UNIT);
    assert_eq!(c.token.balance(&tesoreria_ext), 100_000 * UNIT);
}

#[test]
fn pago_programado_respeta_timelock_y_liquida_al_vencer() {
    let c = setup();
    let p = c.guard.schedule_payment(
        &c.supplier,
        &(15_000 * UNIT),
        &String::from_str(&c.env, "HITO-01"),
        &3_600,
    );
    assert_eq!(p.status, Status::Scheduled);
    assert_eq!(c.token.balance(&c.supplier), 0);

    let antes = c.guard.try_execute_scheduled(&p.id);
    assert_eq!(antes, Err(Ok(soroban_sdk::Error::from_contract_error(Error::TimelockActive as u32))));

    c.env.ledger().with_mut(|li| li.timestamp += 3_601);
    let pagado = c.guard.execute_scheduled(&p.id);
    assert_eq!(pagado.status, Status::Paid);
    assert_eq!(c.token.balance(&c.supplier), 15_000 * UNIT);
}

#[test]
fn expirar_pago_vencido_libera_folio_y_reserva() {
    let c = setup();
    let inv = String::from_str(&c.env, "EXP-99");
    let p = c.guard.request_payment(&c.supplier, &(70_000 * UNIT), &inv);
    assert_eq!(p.status, Status::Pending);

    c.env.ledger().with_mut(|li| li.timestamp += PENDING_TTL_SECS + 10);
    let exp = c.guard.expire_payment(&p.id);
    assert_eq!(exp.status, Status::Expired);
    assert_eq!(c.guard.invoice_payment(&inv), None);
}

#[test]
fn rotacion_de_operador_actualiza_agente() {
    let c = setup();
    let nuevo = Address::generate(&c.env);
    let res = c.guard.rotate_operator(&nuevo);
    assert_eq!(res, nuevo);
    assert_eq!(c.guard.get_config().operator, nuevo);
}

#[test]
fn cancelar_pago_programado_libera_reserva_y_folio() {
    let c = setup();
    let inv = String::from_str(&c.env, "HITO-CANCEL");
    let p = c.guard.schedule_payment(&c.supplier, &(20_000 * UNIT), &inv, &7_200);
    let cancelado = c.guard.cancel_scheduled(&p.id);
    assert_eq!(cancelado.status, Status::Rejected);
    assert_eq!(c.guard.invoice_payment(&inv), None);
}

#[test]
fn ampliacion_temporal_diaria_expira_al_siguiente_dia() {
    let c = setup();
    for f in ["B1", "B2", "B3"] {
        c.guard.request_payment(&c.supplier, &(45_000 * UNIT), &String::from_str(&c.env, f));
    }
    let nuevo_tope = c.guard.grant_daily_boost(&(40_000 * UNIT));
    assert_eq!(nuevo_tope, 190_000 * UNIT);

    let p = c.guard.request_payment(&c.supplier, &(45_000 * UNIT), &String::from_str(&c.env, "B4"));
    assert_eq!(p.status, Status::Paid);
    assert_eq!(c.guard.get_config().daily_limit, 150_000 * UNIT);
}

#[test]
fn transferencia_de_admin_en_dos_pasos_evita_perdida_de_llave() {
    let c = setup();
    let nuevo_admin = Address::generate(&c.env);
    c.guard.propose_admin(&nuevo_admin);
    let aceptado = c.guard.accept_admin();
    assert_eq!(aceptado, nuevo_admin);
    assert_eq!(c.guard.get_config().admin, nuevo_admin);
}

#[test]
fn pago_con_retencion_fiscal_divide_neto_e_impuesto() {
    let c = setup();
    let boveda_sat = Address::generate(&c.env);
    // Flete por 25,000 MXNe con retención de 4% IVA (400 bps = 1,000 MXNe retenidos, 24,000 MXNe netos)
    let p = c.guard.pay_with_withholding(
        &c.supplier,
        &(25_000 * UNIT),
        &400,
        &boveda_sat,
        &String::from_str(&c.env, "CFDI-RET-01"),
    );
    assert_eq!(p.status, Status::Paid);
    assert_eq!(p.amount, 24_000 * UNIT);
    assert_eq!(c.token.balance(&c.supplier), 24_000 * UNIT);
    assert_eq!(c.token.balance(&boveda_sat), 1_000 * UNIT);
    assert_eq!(c.guard.spent_today(), 25_000 * UNIT);

    let invalido = c.guard.try_pay_with_withholding(
        &c.supplier,
        &(10_000 * UNIT),
        &5_000,
        &boveda_sat,
        &String::from_str(&c.env, "CFDI-RET-02"),
    );
    assert_eq!(
        invalido,
        Err(Ok(soroban_sdk::Error::from_contract_error(Error::InvalidBasisPoints as u32)))
    );
}

#[test]
fn plan_de_parcialidades_liquida_anticipo_y_tramos_en_escrow() {
    let c = setup();
    // Contrato de 60,000 MXNe: 30% anticipo inmediato (18,000) + 2 parcialidades de 21,000
    let plan = c.guard.open_installment_plan(
        &c.supplier,
        &(60_000 * UNIT),
        &3_000,
        &2,
        &String::from_str(&c.env, "OBRA-60K"),
    );
    assert_eq!(c.token.balance(&c.supplier), 18_000 * UNIT);
    assert_eq!(plan.remaining, 42_000 * UNIT);
    assert!(plan.active);

    let p1 = c.guard.pay_installment(&plan.id);
    assert_eq!(p1.paid_count, 1);
    assert_eq!(c.token.balance(&c.supplier), 39_000 * UNIT);

    let p2 = c.guard.pay_installment(&plan.id);
    assert_eq!(p2.paid_count, 2);
    assert!(!p2.active);
    assert_eq!(c.token.balance(&c.supplier), 60_000 * UNIT);

    let extra = c.guard.try_pay_installment(&plan.id);
    assert_eq!(
        extra,
        Err(Ok(soroban_sdk::Error::from_contract_error(Error::InstallmentCompleted as u32)))
    );
}

#[test]
fn liquidar_con_nota_de_credito_descuenta_y_libera_reserva() {
    let c = setup();
    let p = c.guard.request_payment(&c.supplier, &(80_000 * UNIT), &String::from_str(&c.env, "FAC-80K"));
    assert_eq!(p.status, Status::Pending);

    let cn = String::from_str(&c.env, "NC-EGRESO-12K");
    let liquidado = c.guard.settle_with_credit_note(&p.id, &(12_000 * UNIT), &cn);
    assert_eq!(liquidado.status, Status::Paid);
    assert_eq!(liquidado.amount, 68_000 * UNIT);
    assert_eq!(c.token.balance(&c.supplier), 68_000 * UNIT);

    // La misma nota de crédito no puede reutilizarse en otra factura
    let p2 = c.guard.request_payment(&c.supplier, &(60_000 * UNIT), &String::from_str(&c.env, "FAC-60K"));
    let dup = c.guard.try_settle_with_credit_note(&p2.id, &(5_000 * UNIT), &cn);
    assert_eq!(dup, Err(Ok(soroban_sdk::Error::from_contract_error(Error::DuplicateInvoice as u32))));
}

#[test]
fn pronto_pago_transfiere_neto_y_segrega_ahorro_financiero() {
    let c = setup();
    let ahorro_cfo = Address::generate(&c.env);
    // Factura nominal de 40,000 MXNe con 5% de descuento por pronto pago (500 bps -> 2,000 ahorro, 38,000 proveedor)
    let p = c.guard.pay_early_discounted(
        &c.supplier,
        &(40_000 * UNIT),
        &500,
        &ahorro_cfo,
        &String::from_str(&c.env, "PRONTO-40K"),
    );
    assert_eq!(p.status, Status::Paid);
    assert_eq!(p.amount, 38_000 * UNIT);
    assert_eq!(c.token.balance(&c.supplier), 38_000 * UNIT);
    assert_eq!(c.token.balance(&ahorro_cfo), 2_000 * UNIT);
}
