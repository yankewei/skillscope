use crate::codex::doctor::DoctorReport;
use crate::codex::scan::ScanResult;
use crate::stats::{InvocationTypeStat, SkillStat};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ScanRequest {
    pub rescan: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub type ScanResponse = ScanResult;
pub type DoctorResponse = DoctorReport;
pub type SkillStatsResponse = Vec<SkillStat>;
pub type InvocationTypeStatsResponse = Vec<InvocationTypeStat>;
