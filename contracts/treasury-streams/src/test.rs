#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, String, Vec};

const UNIT: i128 = 10_000_000;

struct Ctx {
    env: Env,
    streams: TreasuryStreamsClient<'static>,
    token: token::Client<'static>,
    funder: Address,
    worker: Address,
    supplier: Address,
    vault_b: Address,
}

fn setup() -> Ctx {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let token_addr = sac.address();

    let cid = env.register(TreasuryStreams, (admin, operator, token_addr.clone()));
    let funder = Address::generate(&env);
    let sac_admin = StellarAssetClient::new(&env, &token_addr);
    sac_admin.mint(&cid, &(5_000_000 * UNIT));
    sac_admin.mint(&funder, &(1_000_000 * UNIT));

    Ctx {
        streams: TreasuryStreamsClient::new(&env, &cid),
        token: token::Client::new(&env, &token_addr),
        funder,
        worker: Address::generate(&env),
        supplier: Address::generate(&env),
        vault_b: Address::generate(&env),
        env,
    }
}

#[test]
fn test_01_fund_payroll_pool() {
    let c = setup();
    let pool = c.streams.fund_payroll_pool(&c.funder, &(50_000 * UNIT));
    assert_eq!(pool, 50_000 * UNIT);
}

#[test]
fn test_02_open_salary_stream() {
    let c = setup();
    let st = c.streams.open_salary_stream(&c.worker, &(30_000 * UNIT), &(5_000 * UNIT));
    assert!(st.active);
    assert_eq!(c.token.balance(&c.worker), 5_000 * UNIT);
}

#[test]
fn test_03_claim_accrued_salary() {
    let c = setup();
    let st = c.streams.open_salary_stream(&c.worker, &(30_000 * UNIT), &(2_000 * UNIT));
    let updated = c.streams.claim_accrued_salary(&st.id, &(10_000 * UNIT));
    assert_eq!(updated.remaining, 20_000 * UNIT);
    assert_eq!(c.token.balance(&c.worker), 12_000 * UNIT);
}

#[test]
fn test_04_disburse_aguinaldo_bonus() {
    let c = setup();
    let ag = c.streams.disburse_aguinaldo_bonus(&c.worker, &(30_000 * UNIT), &365);
    assert_eq!(ag, 15_000 * UNIT);
    assert_eq!(c.token.balance(&c.worker), 15_000 * UNIT);
}

#[test]
fn test_05_settle_severance_finiquito() {
    let c = setup();
    let st = c.streams.open_salary_stream(&c.worker, &(20_000 * UNIT), &(1_000 * UNIT));
    let fin = c.streams.settle_severance_finiquito(&st.id, &(8_000 * UNIT));
    assert_eq!(fin, 28_000 * UNIT);
    assert_eq!(c.token.balance(&c.worker), 29_000 * UNIT);
}

#[test]
fn test_06_stake_and_release_supplier_bond() {
    let c = setup();
    let b = c.streams.stake_supplier_bond(&c.funder, &c.supplier, &(40_000 * UNIT), &String::from_str(&c.env, "OC-101"));
    let rel = c.streams.release_supplier_bond(&b.id);
    assert!(!rel.active);
    assert_eq!(c.token.balance(&c.supplier), 40_000 * UNIT);
}

#[test]
fn test_07_slash_supplier_bond() {
    let c = setup();
    let b = c.streams.stake_supplier_bond(&c.funder, &c.supplier, &(20_000 * UNIT), &String::from_str(&c.env, "OC-102"));
    let pen = c.streams.slash_supplier_bond(&b.id, &2_500, &c.vault_b);
    assert_eq!(pen, 5_000 * UNIT);
    assert_eq!(c.token.balance(&c.vault_b), 5_000 * UNIT);
    assert_eq!(c.token.balance(&c.supplier), 15_000 * UNIT);
}

#[test]
fn test_08_warranty_holdback_and_claim() {
    let c = setup();
    let w = c.streams.pay_retention_warranty_holdback(&c.supplier, &(100_000 * UNIT), &1_000);
    assert_eq!(c.token.balance(&c.supplier), 90_000 * UNIT);
    let done = c.streams.claim_warranty_holdback(&w.id);
    assert!(!done.active);
    assert_eq!(c.token.balance(&c.supplier), 100_000 * UNIT);
}

#[test]
fn test_09_factoring_advance_and_repay() {
    let c = setup();
    let rec = c.streams.advance_factoring_invoice(&c.supplier, &c.vault_b, &(50_000 * UNIT), &200);
    assert_eq!(c.token.balance(&c.supplier), 49_000 * UNIT);
    assert_eq!(c.token.balance(&c.vault_b), 1_000 * UNIT);
    let rep = c.streams.repay_factoring_line(&c.funder, &rec.id);
    assert!(rep.repaid);
}

#[test]
fn test_10_revolving_sub_budget_draw_and_sweep() {
    let c = setup();
    let sb = c.streams.open_revolving_sub_budget(&c.worker, &(50_000 * UNIT), &(10_000 * UNIT));
    assert_eq!(c.token.balance(&c.worker), 10_000 * UNIT);
    let sb2 = c.streams.draw_sub_budget(&sb.id, &(15_000 * UNIT), &c.supplier);
    assert_eq!(sb2.remaining, 25_000 * UNIT);
    assert_eq!(c.token.balance(&c.supplier), 15_000 * UNIT);
    let swept = c.streams.sweep_unused_sub_budget(&sb.id, &c.vault_b);
    assert_eq!(swept, 25_000 * UNIT);
    assert_eq!(c.token.balance(&c.vault_b), 25_000 * UNIT);
}

#[test]
fn test_11_provision_iva_isr_and_imss() {
    let c = setup();
    let tax = c.streams.provision_iva_isr_bucket(&c.vault_b, &(100_000 * UNIT), &1_600, &1_000);
    assert_eq!(tax, 26_000 * UNIT);
    let imss = c.streams.remit_imss_infonavit_quota(&c.vault_b, &12, &(200_000 * UNIT), &1_500);
    assert_eq!(imss, 30_000 * UNIT);
    assert_eq!(c.token.balance(&c.vault_b), 56_000 * UNIT);
}

#[test]
fn test_12_ptu_and_viaticos_reconciliation() {
    let c = setup();
    let mut wks = Vec::new(&c.env);
    wks.push_back(c.worker.clone());
    wks.push_back(c.supplier.clone());
    let share = c.streams.distribute_ptu_profit_share(&wks, &(40_000 * UNIT));
    assert_eq!(share, 20_000 * UNIT);

    let tk_rec = c.streams.issue_travel_viaticos(&c.worker, &(2_000 * UNIT), &4);
    assert_eq!(tk_rec.disbursed, 8_000 * UNIT);
    let closed = c.streams.reconcile_viaticos_surplus(&c.funder, &tk_rec.id, &(1_500 * UNIT));
    assert!(closed.closed);
}

#[test]
fn test_13_letter_of_credit_dispute_and_fx_hedge() {
    let c = setup();
    let lc = c.streams.open_letter_of_credit(&c.supplier, &(90_000 * UNIT), &(3_000 * UNIT), &c.worker);
    assert_eq!(c.token.balance(&c.worker), 3_000 * UNIT);
    let settled = c.streams.settle_letter_of_credit(&lc.id);
    assert!(settled.settled);
    assert_eq!(c.token.balance(&c.supplier), 87_000 * UNIT);

    let sup_part = c.streams.resolve_invoice_dispute_split(&c.supplier, &c.vault_b, &(20_000 * UNIT), &6_000);
    assert_eq!(sup_part, 12_000 * UNIT);

    let hp = c.streams.lock_fx_hedge_margin(&c.vault_b, &(100_000 * UNIT), &1_000);
    assert_eq!(hp.margin, 10_000 * UNIT);
    let hp_done = c.streams.settle_fx_hedge_pnl(&hp.id, &c.worker, &(4_500 * UNIT));
    assert!(hp_done.settled);
}
