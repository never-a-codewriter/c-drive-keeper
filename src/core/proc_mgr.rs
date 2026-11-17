use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::path::Path;
use walkdir::WalkDir;
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{CloseHandle, ERROR_MORE_DATA, HANDLE},
        System::{
            RestartManager::{
                RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
                RM_PROCESS_INFO,
            },
            Threading::{
                OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
            },
        },
    },
};

pub struct ProcMgr;

impl ProcMgr {
    pub fn check_locking_processes(path: &Path) -> Result<Vec<u32>> {
        unsafe {
            let mut session_handle: u32 = 0;
            let mut session_key_buf = [0u16; CCH_RM_SESSION_KEY as usize];
            let session_key = PWSTR::from_raw(session_key_buf.as_mut_ptr());

            let res = RmStartSession(&mut session_handle, 0, session_key);
            if res != 0 {
                return Err(anyhow!("无法启动 Restart Manager 会话: {}", res));
            }

            let result = (|| -> Result<Vec<u32>> {
                let path_str = path.to_string_lossy();
                let mut wide_path: Vec<u16> = path_str.encode_utf16().collect();
                wide_path.push(0);

                let paths = [PCWSTR::from_raw(wide_path.as_ptr())];
                let res = RmRegisterResources(session_handle, Some(&paths), None, None);
                if res != 0 {
                    return Err(anyhow!("无法注册资源: {}", res));
                }

                let mut array_len_needed: u32 = 0;
                let mut array_len = 0;
                let mut reboot_reasons = 0;

                let _ = RmGetList(
                    session_handle,
                    &mut array_len_needed,
                    &mut array_len,
                    None,
                    &mut reboot_reasons,
                );

                if array_len_needed == 0 {
                    return Ok(Vec::new());
                }

                let mut process_info = vec![RM_PROCESS_INFO::default(); array_len_needed as usize];
                array_len = array_len_needed;

                let res = RmGetList(
                    session_handle,
                    &mut array_len_needed,
                    &mut array_len,
                    Some(process_info.as_mut_ptr()),
                    &mut reboot_reasons,
                );

                if res != 0 && res != ERROR_MORE_DATA.0 {
                    return Err(anyhow!("获取进程列表失败: {}", res));
                }

                let pids: Vec<u32> = process_info
                    .iter()
                    .take(array_len as usize)
                    .map(|info| info.Process.dwProcessId)
                    .collect();

                Ok(pids)
            })();

            let _ = RmEndSession(session_handle);
            result
        }
    }

    pub fn check_locking_processes_dir(path: &Path) -> Result<Vec<u32>> {
        if path.is_file() {
            return Self::check_locking_processes(path);
        }
        if !path.is_dir() {
            return Ok(Vec::new());
        }

        let mut pids: HashSet<u32> = HashSet::new();
        let candidates: Vec<_> = WalkDir::new(path)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                let name = e.file_name().to_string_lossy();
                name.eq_ignore_ascii_case("LOCK")
                    || name.eq_ignore_ascii_case(".lock")
                    || name.eq_ignore_ascii_case("LOCKFILE")
            })
            .take(50)
            .map(|e| e.into_path())
            .collect();

        for f in candidates {
            if let Ok(list) = Self::check_locking_processes(&f) {
                for pid in list {
                    pids.insert(pid);
                }
            }
        }

        Ok(pids.into_iter().collect())
    }

    pub fn kill_process(pid: u32) -> Result<()> {
        unsafe {
            let handle: HANDLE = OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            )?;
            if handle.is_invalid() {
                return Err(anyhow!("无法打开进程 {}", pid));
            }

            let res = TerminateProcess(handle, 1);
            let _ = CloseHandle(handle);
            if !res.as_bool() {
                return Err(anyhow!("无法结束进程 {}: (错误码不明)", pid));
            }

            Ok(())
        }
    }
}
