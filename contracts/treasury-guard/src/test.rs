#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, String};

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
