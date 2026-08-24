use serde::{Deserialize, Serialize};

use crate::contract::MissionContract;
use crate::ids::{MissionId, Timestamp};
use crate::route::Route;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Mission {
    pub mission_id: MissionId,
    pub contract: MissionContract,
    pub routes: Vec<Route>,
    pub created_at: Timestamp,
}

impl Mission {
    pub fn new(mission_id: MissionId, contract: MissionContract) -> Self {
        Self {
            mission_id,
            contract,
            routes: Vec::new(),
            created_at: Timestamp::now(),
        }
    }

    pub fn add_route(&mut self, route: Route) {
        self.routes.push(route);
    }
}
