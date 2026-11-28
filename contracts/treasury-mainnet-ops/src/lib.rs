#![no_std]
//! TreasuryMainnetOps — operaciones fiscales (IVA/ISR), nómina en lote y barrido de reserva
//! en producción sobre Stellar Public Mainnet para TesorerIA.

use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env, Symbol};

#[contract]
pub struct TreasuryMainnetOps;

#[contractimpl]
impl TreasuryMainnetOps {
    pub fn __constructor(env: Env, admin: Address, token: Address) {
        let st = env.storage().instance();
        st.set(&symbol_short!("adm"), &admin);
        st.set(&symbol_short!("tok"), &token);
        st.extend_ttl(100_000, 200_000);
    }

    pub fn pay_with_tax(
        env: Env,
        supplier: Address,
        tax_vault: Address,
        gross: i128,
        tax_bps: u32,
        folio: Symbol,
    ) -> i128 {
        let st = env.storage().instance();
        let adm: Address = st.get(&symbol_short!("adm")).unwrap();
        adm.require_auth();
        assert!(gross > 0 && tax_bps > 0 && tax_bps <= 3500 && !st.has(&folio));
        let tax = (gross * (tax_bps as i128)) / 10_000;
        let net = gross - tax;
        let tok: Address = st.get(&symbol_short!("tok")).unwrap();
        let tk = token::Client::new(&env, &tok);
        let me = env.current_contract_address();
        tk.transfer(&me, &supplier, &net);
        tk.transfer(&me, &tax_vault, &tax);
        st.set(&folio, &gross);
        st.extend_ttl(100_000, 200_000);
        net
    }

    pub fn batch_payroll(
        env: Env,
        w1: Address,
        w2: Address,
        amount_each: i128,
        period: Symbol,
    ) -> i128 {
        let st = env.storage().instance();
        let adm: Address = st.get(&symbol_short!("adm")).unwrap();
        adm.require_auth();
        assert!(amount_each > 0 && !st.has(&period));
        let tok: Address = st.get(&symbol_short!("tok")).unwrap();
        let tk = token::Client::new(&env, &tok);
        let me = env.current_contract_address();
        tk.transfer(&me, &w1, &amount_each);
        tk.transfer(&me, &w2, &amount_each);
        let total = amount_each * 2;
        st.set(&period, &total);
        st.extend_ttl(100_000, 200_000);
        total
    }

    pub fn sweep_reserve(env: Env, dest: Address, amount: i128) -> i128 {
        let st = env.storage().instance();
        let adm: Address = st.get(&symbol_short!("adm")).unwrap();
        adm.require_auth();
        assert!(amount > 0);
        let tok: Address = st.get(&symbol_short!("tok")).unwrap();
        let tk = token::Client::new(&env, &tok);
        let me = env.current_contract_address();
        tk.transfer(&me, &dest, &amount);
        st.extend_ttl(100_000, 200_000);
        tk.balance(&me)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, token::StellarAssetClient};

    #[test]
    fn test_ops_tax_payroll_and_sweep() {
        let env = Env::default();
        env.mock_all_auths();
        let adm = Address::generate(&env);
        let sup = Address::generate(&env);
        let sat = Address::generate(&env);
        let issuer = Address::generate(&env);
        let sac = env.register_stellar_asset_contract_v2(issuer);
        let tok = sac.address();

        let cid = env.register(TreasuryMainnetOps, (adm.clone(), tok.clone()));
        StellarAssetClient::new(&env, &tok).mint(&cid, &50_000_000i128);

        let cli = TreasuryMainnetOpsClient::new(&env, &cid);
        assert_eq!(cli.pay_with_tax(&sup, &sat, &10_000_000i128, &1000, &symbol_short!("RET01")), 9_000_000i128);
        assert_eq!(cli.batch_payroll(&sup, &sat, &5_000_000i128, &symbol_short!("Q22")), 10_000_000i128);
        assert_eq!(cli.sweep_reserve(&adm, &5_000_000i128), 25_000_000i128);
    }
}
