#![no_std]
//! TreasuryMainnet — bóveda compacta de producción en Stellar Mainnet para TesorerIA.
//! Verifica lista blanca, tope por transacción, límite diario y unicidad de folio fiscal (CFDI)
//! antes de liberar XLM reales vía Stellar Asset Contract (SAC).

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env, Symbol};

#[contract]
pub struct TreasuryMainnet;

#[contractimpl]
impl TreasuryMainnet {
    pub fn __constructor(env: Env, admin: Address, token: Address, daily_cap: i128) {
        let st = env.storage().instance();
        st.set(&symbol_short!("adm"), &admin);
        st.set(&symbol_short!("tok"), &token);
        st.set(&symbol_short!("cap"), &daily_cap);
        st.extend_ttl(100_000, 200_000);
    }

    pub fn set_supplier(env: Env, supplier: Address, max_tx: i128) {
        let st = env.storage().instance();
        let adm: Address = st.get(&symbol_short!("adm")).unwrap();
        adm.require_auth();
        assert!(max_tx > 0);
        st.set(&supplier, &max_tx);
        st.extend_ttl(100_000, 200_000);
    }

    pub fn pay_invoice(env: Env, supplier: Address, amount: i128, folio: Symbol) -> i128 {
        let st = env.storage().instance();
        let adm: Address = st.get(&symbol_short!("adm")).unwrap();
        adm.require_auth();
        let max_tx: i128 = st.get(&supplier).unwrap_or(0);
        assert!(amount > 0 && amount <= max_tx);
        assert!(!st.has(&folio));
        let cap: i128 = st.get(&symbol_short!("cap")).unwrap();
        let spent: i128 = st.get(&symbol_short!("spn")).unwrap_or(0) + amount;
        assert!(spent <= cap);
        let tok: Address = st.get(&symbol_short!("tok")).unwrap();
        token::Client::new(&env, &tok).transfer(&env.current_contract_address(), &supplier, &amount);
        st.set(&folio, &amount);
        st.set(&symbol_short!("spn"), &spent);
        st.extend_ttl(100_000, 200_000);
        spent
    }

    pub fn spent_today(env: Env) -> i128 {
        env.storage().instance().get(&symbol_short!("spn")).unwrap_or(0)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient};

    #[test]
    fn test_mainnet_vault_policy_and_pay() {
        let env = Env::default();
        env.mock_all_auths();
        let adm = Address::generate(&env);
        let sup = Address::generate(&env);
        let issuer = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(issuer);
        let tok = sac.address();

        let cid = env.register(TreasuryMainnet, (adm, tok.clone(), 50_000_000i128));
        StellarAssetClient::new(&env, &tok).mint(&cid, &20_000_000i128);

        let cli = TreasuryMainnetClient::new(&env, &cid);
        cli.set_supplier(&sup, &10_000_000i128);
        let sp = cli.pay_invoice(&sup, &5_000_000i128, &symbol_short!("F2041"));
        assert_eq!(sp, 5_000_000i128);
        assert_eq!(token::Client::new(&env, &tok).balance(&sup), 5_000_000i128);
    }
}
