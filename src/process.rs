//! 프로세스 열거/해석 (프로세스별 루프백 캡처 대상 선택용).

use std::collections::HashSet;

use anyhow::{Result, bail};
use sysinfo::{ProcessesToUpdate, System};

fn collect() -> Vec<(u32, String, Option<u32>)> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes()
        .iter()
        .map(|(pid, p)| {
            (
                pid.as_u32(),
                p.name().to_string_lossy().to_string(),
                p.parent().map(|x| x.as_u32()),
            )
        })
        .collect()
}

/// (pid, 이름) 목록 — 이름 오름차순.
pub fn list() -> Vec<(u32, String)> {
    let mut v: Vec<(u32, String)> = collect()
        .into_iter()
        .map(|(pid, name, _)| (pid, name))
        .collect();
    v.sort_by(|a, b| {
        a.1.to_lowercase()
            .cmp(&b.1.to_lowercase())
            .then(a.0.cmp(&b.0))
    });
    v
}

/// PID(숫자) 또는 프로세스명(부분일치) → 캡처할 PID.
/// 이름이 여러 개면 "루트"(부모가 같은 이름 집합 밖) 프로세스를 고른다 — 브라우저처럼
/// 오디오를 자식(오디오 서비스) 프로세스에서 내는 경우 루트+트리포함으로 잡기 위함.
pub fn resolve(query: &str) -> Result<u32> {
    let q = query.trim();
    if let Ok(pid) = q.parse::<u32>() {
        return Ok(pid);
    }
    let want = q.to_lowercase();
    let all = collect();
    let matches: Vec<&(u32, String, Option<u32>)> = all
        .iter()
        .filter(|(_, name, _)| name.to_lowercase().contains(&want))
        .collect();
    if matches.is_empty() {
        bail!("'{query}' 와 일치하는 실행 중 프로세스를 못 찾음 (`processes` 로 목록 확인)");
    }
    let match_pids: HashSet<u32> = matches.iter().map(|m| m.0).collect();
    let root = matches
        .iter()
        .filter(|(_, _, parent)| parent.map_or(true, |pp| !match_pids.contains(&pp)))
        .min_by_key(|m| m.0);
    let chosen = root.or_else(|| matches.iter().min_by_key(|m| m.0)).unwrap();
    Ok(chosen.0)
}

/// pid → 이름(표시용).
pub fn name_of(pid: u32) -> Option<String> {
    collect()
        .into_iter()
        .find(|(p, _, _)| *p == pid)
        .map(|(_, name, _)| name)
}
