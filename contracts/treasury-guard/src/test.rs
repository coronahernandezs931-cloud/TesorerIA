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
