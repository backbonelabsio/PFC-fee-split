use cw2::{get_contract_version, set_contract_version};
use std::collections::HashSet;
use std::str::FromStr;

/// Contract name that is used for migration.
const CONTRACT_NAME: &str = "pfc-fee-split";
/// Contract version that is used for migration.
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    to_binary, Binary, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Response, StdResult, WasmMsg,
};

use crate::error::ContractError;
use crate::handler::exec as ExecHandler;
use crate::handler::query as QueryHandler;
use crate::migrations::ConfigV100;
use crate::state;
use crate::state::{ADMIN, ALLOCATION_HOLDINGS, CONFIG};

use crate::error::ContractError::SendTypeInvalid;
use pfc_fee_split::fee_split_msg::{
    AllocationHolding, ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg, SendType,
};

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    mut deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    CONFIG.save(
        deps.storage,
        &state::Config {
            this: deps.api.addr_validate(env.contract.address.as_str())?,
            gov_contract: deps.api.addr_validate(msg.gov_contract.as_str())?,
            new_gov_contract: None,
            change_gov_contract_by_height: None,
        },
    )?;

    if msg.allocation.is_empty() {
        return Err(ContractError::NoFeesError {});
    }
    let dupe_check: HashSet<String> = msg.allocation.iter().map(|v| v.name.clone()).collect();
    if dupe_check.len() != msg.allocation.len() {
        return Err(ContractError::FundAllocationNotUnique {});
    }
    for row in msg.allocation {
        if row.send_after.denom.trim().is_empty() {
            return Err(ContractError::InvalidCoin {
                coin: row.send_after,
            });
        }
        let send_type_verified =
            SendType::from_str(&row.send_type).map_err(|_| SendTypeInvalid {
                send_type: row.send_type.clone(),
            })?;
        if row.allocation == 0 {
            return Err(ContractError::AllocationZero {});
        }

        let allocation_holding: AllocationHolding = AllocationHolding {
            name: row.name.clone(),
            contract: deps.api.addr_validate(row.contract.as_str())?,
            allocation: row.allocation,
            send_after: row.send_after,
            send_type: send_type_verified,
            balance: vec![],
        };
        ALLOCATION_HOLDINGS.save(deps.storage, row.name.clone(), &allocation_holding)?
    }

    let admin = deps.api.addr_validate(&msg.gov_contract)?;
    ADMIN.set(deps.branch(), Some(admin))?;

    let mut res = Response::new();
    if let Some(hook) = msg.init_hook {
        res = res.add_message(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: hook.contract_addr,
            msg: hook.msg,
            funds: vec![],
        }));
    }

    Ok(res)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Deposit { flush } => ExecHandler::execute_deposit(deps, env, info, flush),
        ExecuteMsg::AddAllocationDetail {
            name,
            contract,
            allocation,
            send_after,
            send_type,
        } => ExecHandler::execute_add_allocation_detail(
            deps, env, info, name, contract, allocation, send_after, send_type,
        ),

        ExecuteMsg::RemoveAllocationDetail { name } => {
            ExecHandler::execute_remove_allocation_detail(deps, env, info, name)
        }

        ExecuteMsg::TransferGovContract {
            gov_contract,
            blocks,
        } => ExecHandler::execute_update_gov_contract(deps, env, info, gov_contract, blocks),
        ExecuteMsg::AcceptGovContract {} => {
            ExecHandler::execute_accept_gov_contract(deps, env, info)
        }
        ExecuteMsg::ModifyAllocationDetail {
            name,
            contract,
            allocation,
            send_after,
            send_type,
        } => ExecHandler::execute_modify_allocation_detail(
            deps, env, info, name, contract, allocation, send_after, send_type,
        ),
        ExecuteMsg::Reconcile {} => ExecHandler::execute_reconcile(deps, env, info),
        ExecuteMsg::AddToFlushWhitelist { address } => {
            ExecHandler::execute_add_flush_whitelist(deps, env, info, address)
        }
        ExecuteMsg::RemoveFromFlushWhitelist { address } => {
            ExecHandler::execute_remove_flush_whitelist(deps, env, info, address)
        }
    }
}
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Ownership {} => to_binary(&QueryHandler::query_gov_contract(deps)?),
        QueryMsg::Allocations { start_after, limit } => {
            to_binary(&QueryHandler::query_allocations(deps, start_after, limit)?)
        }

        QueryMsg::Allocation { name } => to_binary(&QueryHandler::query_allocation(deps, name)?),
        QueryMsg::FlushWhitelist {} => to_binary(&QueryHandler::query_flush_whitelist(deps)?),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    let contract_version = get_contract_version(deps.storage)?;

    match contract_version.contract.as_ref() {
        #[allow(clippy::single_match)]
        "pfc-fee-split" => match contract_version.version.as_ref() {
            "0.1.1" => {
                let config_v100 = ConfigV100::load(deps.storage)?;

                CONFIG.save(deps.storage, &config_v100.migrate_from())?;
            }

            _ => {}
        },
        _ => {
            return Err(ContractError::MigrationError {
                current_name: contract_version.contract,
                current_version: contract_version.version,
            })
        }
    }

    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    Ok(Response::new()
        .add_attribute("previous_contract_name", &contract_version.contract)
        .add_attribute("previous_contract_version", &contract_version.version)
        .add_attribute("new_contract_name", CONTRACT_NAME)
        .add_attribute("new_contract_version", CONTRACT_VERSION))
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{CosmosMsg, SubMsg, WasmMsg};

    mod instantiate {
        use super::*;
        use crate::contract::instantiate;
        use cosmwasm_std::{coin, Api, Binary};

        use crate::error::ContractError;
        use crate::handler::query::query_allocation;
        use crate::test_helpers::{
            one_allocation, ALLOCATION_1, ALLOCATION_2, CREATOR, DENOM_1, GOV_CONTRACT,
        };
        use pfc_fee_split::fee_split_msg::{
            AllocationDetail, AllocationHolding, InitHook, InstantiateMsg, SendType,
        };

        #[test]
        fn basic() {
            let mut deps = mock_dependencies();
            let hook_msg = Binary::from(r#"{"some": 123}"#.as_bytes());
            let instantiate_msg = InstantiateMsg {
                name: "Hook Test".to_string(),

                init_hook: Some(InitHook {
                    contract_addr: String::from("hook_dest"),
                    msg: hook_msg.clone(),
                }),
                gov_contract: String::from(GOV_CONTRACT),
                allocation: one_allocation(),
            };

            let info = mock_info(CREATOR, &[]);
            let env = mock_env();
            let res = instantiate(deps.as_mut(), env, info, instantiate_msg).unwrap();
            assert_eq!(
                res.messages,
                vec![SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
                    contract_addr: String::from("hook_dest"),
                    msg: hook_msg,
                    funds: vec![],
                }))]
            );
            assert_eq!(
                query_allocation(deps.as_ref(), ALLOCATION_1.into())
                    .unwrap()
                    .unwrap(),
                AllocationHolding {
                    name: ALLOCATION_1.to_string(),
                    contract: deps.api.addr_validate("allocation_1_addr").unwrap(),
                    allocation: 1,
                    send_after: coin(1_000u128, DENOM_1),
                    send_type: SendType::WALLET,
                    balance: vec![]
                }
            );
            let instantiate_no_allocation_msg = InstantiateMsg {
                name: "FAIL".to_string(),
                init_hook: None,
                gov_contract: String::from(GOV_CONTRACT),
                allocation: vec![],
            };
            let info = mock_info(CREATOR, &[]);
            let env = mock_env();
            if let Some(err) =
                instantiate(deps.as_mut(), env, info, instantiate_no_allocation_msg).err()
            {
                match err {
                    ContractError::NoFeesError { .. } => {}
                    _ => assert!(false, "Invalid Error type {}", err),
                }
            } else {
                assert!(false, "should have failed")
            }
        }

        #[test]
        fn dupe_holdings() {
            let mut deps = mock_dependencies();
            let instantiate_msg = InstantiateMsg {
                name: "Dupe Allocation Test".to_string(),

                init_hook: None,
                gov_contract: String::from(GOV_CONTRACT),
                allocation: vec![
                    AllocationDetail {
                        name: ALLOCATION_1.to_string(),
                        contract: "allocation_1_addr".to_string(),
                        allocation: 1,
                        send_after: coin(1_000u128, DENOM_1),
                        send_type: "Wallet".to_string(),
                    },
                    AllocationDetail {
                        name: ALLOCATION_2.to_string(),
                        contract: "allocation_2_addr".to_string(),
                        allocation: 1,
                        send_after: coin(1_0000_000u128, DENOM_1),
                        send_type: "Wallet".to_string(),
                    },
                    AllocationDetail {
                        name: ALLOCATION_1.to_string(),
                        contract: "allocation_3_addr".to_string(),
                        allocation: 3,
                        send_after: coin(1_0000_000u128, DENOM_1),
                        send_type: "Wallet".to_string(),
                    },
                ],
            };

            let info = mock_info(CREATOR, &[]);
            let env = mock_env();
            let err = instantiate(deps.as_mut(), env, info, instantiate_msg)
                .err()
                .unwrap();
            match err {
                ContractError::FundAllocationNotUnique {} => {}
                _ => {
                    assert!(
                        false,
                        "this should have returned an FundAllocationNotUnique error"
                    )
                }
            }
        }
    }
}
