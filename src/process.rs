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

/// 이름별로 묶은 (이름, 개수) — 이름 오름차순. 같은 앱(예: chrome.exe 탭 다수)이 한 줄로.
pub fn list_named() -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (_, name) in list() {
        *counts.entry(name).or_insert(0) += 1;
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    v
}

/// 지금 **오디오 세션이 있는(소리를 내고 있거나 최근에 낸)** 앱만 이름별로 묶어서 (이름, 세션수).
/// 수백 개 프로세스 대신 chrome·spotify 같은 몇 개만 나온다. 실패하면 빈 벡터.
pub fn list_audio() -> Vec<(String, usize)> {
    let pids = match audio_session_pids() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    if pids.is_empty() {
        return Vec::new();
    }
    let names: std::collections::HashMap<u32, String> = collect()
        .into_iter()
        .map(|(pid, name, _)| (pid, name))
        .collect();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for pid in pids {
        if let Some(name) = names.get(&pid) {
            *counts.entry(name.clone()).or_insert(0) += 1;
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    v
}

/// 기본 출력장치의 오디오 세션을 열거해, 소리를 내는 프로세스 PID들을 모은다(만료 세션·시스템(0) 제외).
fn audio_session_pids() -> Result<Vec<u32>> {
    use wasapi::{Direction, SessionState};
    let _ = wasapi::initialize_mta();
    let device = wasapi::DeviceEnumerator::new()?.get_default_device(&Direction::Render)?;
    let manager = device.get_iaudiosessionmanager()?;
    let enumerator = manager.get_audiosessionenumerator()?;
    let count = enumerator.get_count()?;
    let mut pids = Vec::new();
    for i in 0..count {
        let Ok(session) = enumerator.get_session(i) else {
            continue;
        };
        // 만료된(프로세스 종료) 세션은 제외
        if let Ok(SessionState::Expired) = session.get_state() {
            continue;
        }
        if let Ok(pid) = session.get_process_id() {
            if pid != 0 {
                pids.push(pid);
            }
        }
    }
    Ok(pids)
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
