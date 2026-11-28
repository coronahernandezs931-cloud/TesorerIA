#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{testutils::Address as _, token::StellarAssetClient, Address, Env, Vec};

const UNIT: i128 = 10_000_000;

struct Ctx {
    env: Env,
    credit: TreasuryCreditClient<'static>,
    token: token::Client<'static>,
    payer: Address,
    party_a: Address,
    party_b: Address,
    vault_x: Address,
}

fn setup() -> Ctx {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let issuer = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(issuer);
    let token_addr = sac.address();

    let cid = env.register(TreasuryCredit, (admin, operator, token_addr.clone()));
    let payer = Address::generate(&env);
    let sac_admin = StellarAssetClient::new(&env, &token_addr);
    sac_admin.mint(&cid, &(8_000_000 * UNIT));
    sac_admin.mint(&payer, &(2_000_000 * UNIT));

    Ctx {
        credit: TreasuryCreditClient::new(&env, &cid),
        token: token::Client::new(&env, &token_addr),
        payer,
        party_a: Address::generate(&env),
        party_b: Address::generate(&env),
        vault_x: Address::generate(&env),
        env,
    }
}

#[test]
fn test_credit_loans_and_collateral() {
    let c = setup();
    assert_eq!(c.credit.seed_credit_facility(&c.payer, &(100_000 * UNIT)), 100_000 * UNIT);
    let ln = c.credit.draw_working_capital_loan(&c.party_a, &(50_000 * UNIT), &200, &c.vault_x);
    assert_eq!(c.token.balance(&c.party_a), 49_000 * UNIT);
    let ln2 = c.credit.amortize_loan_installment(&c.payer, &ln.id, &(20_000 * UNIT), &(1_500 * UNIT), &c.vault_x);
    assert_eq!(ln2.outstanding, 30_000 * UNIT);
    let ln3 = c.credit.prepay_loan_with_rebate(&c.payer, &ln.id, &500);
    assert!(!ln3.active);
    assert_eq!(c.credit.liquidate_defaulted_collateral(&c.party_b, &(12_000 * UNIT)), 12_000 * UNIT);
}

#[test]
fn test_cetes_tranches_and_customs_pedimento() {
    let c = setup();
    let tr = c.credit.subscribe_cetes_yield_tranche(&c.party_a, &(80_000 * UNIT));
    assert_eq!(c.credit.harvest_tranche_coupon(&c.payer, &tr.id, &(3_200 * UNIT), &c.vault_x), 3_200 * UNIT);
    assert!(c.credit.redeem_matured_tranche(&c.payer, &tr.id).redeemed);

    let ped = c.credit.open_customs_pedimento_escrow(&c.party_a, &(25_000 * UNIT), &(2_000 * UNIT));
    let cleared = c.credit.pay_aduanal_igi_duty(&ped.id, &c.party_b, &(22_000 * UNIT), &c.vault_x);
    assert!(cleared.cleared);
}

#[test]
fn test_fleet_sales_and_ma_earnout() {
    let c = setup();
    assert_eq!(c.credit.fund_fleet_fuel_card_pool(&c.payer, &c.party_a, &(15_000 * UNIT), &(5_000 * UNIT)), 20_000 * UNIT);
    assert_eq!(c.credit.settle_fleet_toll_tag_batch(&c.party_b, &40, &(120 * UNIT)), 4_800 * UNIT);
    assert_eq!(c.credit.disburse_sales_commission_tier(&c.party_a, &(100_000 * UNIT), &500, &200), 7_000 * UNIT);
    assert_eq!(c.credit.clawback_cancelled_deal_commission(&c.payer, &c.vault_x, &(2_500 * UNIT)), 2_500 * UNIT);

    let eo = c.credit.escrow_m_and_a_earnout(&c.party_a, &(60_000 * UNIT), &(40_000 * UNIT));
    assert!(c.credit.release_earnout_milestone(&eo.id, &7_500, &c.vault_x).closed);
}

#[test]
fn test_governance_dividends_insurance_esg_cfe_royalties() {
    let c = setup();
    let mut dirs = Vec::new(&c.env);
    dirs.push_back(c.party_a.clone());
    dirs.push_back(c.party_b.clone());
    assert_eq!(c.credit.pay_board_retainer_stipend(&dirs, &(8_000 * UNIT)), 16_000 * UNIT);
    assert_eq!(c.credit.declare_shareholder_dividend(&dirs, &(50_000 * UNIT), &1_000, &c.vault_x), 22_500 * UNIT);
    assert_eq!(c.credit.execute_treasury_buyback(&c.party_a, &100, &(250 * UNIT)), 25_000 * UNIT);
    assert_eq!(c.credit.pay_insurance_policy_premium(&c.party_a, &(200_000 * UNIT), &150, &c.party_b, &25), 3_500 * UNIT);
    assert_eq!(c.credit.settle_insurance_claim_deductible(&c.party_a, &(30_000 * UNIT), &(5_000 * UNIT)), 25_000 * UNIT);
    assert_eq!(c.credit.fund_carbon_offset_retirement(&c.vault_x, &120, &(180 * UNIT)), 21_600 * UNIT);
    assert_eq!(c.credit.disburse_union_savings_match(&c.vault_x, &(20_000 * UNIT), &5_000), 10_000 * UNIT);
    assert_eq!(c.credit.settle_utility_cfe_tariff_split(&c.party_b, &4_000, &6_000, &(3 * UNIT)), 30_000 * UNIT);
    assert_eq!(c.credit.pay_royalty_license_waterfall(&c.party_a, &c.party_b, &(100_000 * UNIT), &800, &2_500), 8_000 * UNIT);
}
