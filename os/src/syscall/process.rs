//! Process management syscalls
use alloc::sync::Arc;
use core::mem::size_of;

use crate::{
    config::PAGE_SIZE,
    loader::get_app_data_by_name,
    mm::{copy_to_user, translated_str, MapPermission},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next, TaskControlBlock,
    },
    timer::get_time_us,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let Some(path) = translated_str(token, path) else {
        return -1;
    };
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!(
        "kernel::pid[{}] sys_waitpid [{}]",
        current_task().unwrap().pid.0,
        pid
    );
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let exit_code = inner.children[idx]
            .inner_exclusive_access()
            .exit_code
            .to_ne_bytes();
        if !copy_to_user(
            inner.memory_set.token(),
            exit_code_ptr as *mut u8,
            &exit_code,
        ) {
            return -1;
        }
        let child = inner.children.remove(idx);
        // Removing the final Arc should release the zombie process.
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel:pid[{}] sys_get_time", current_task().unwrap().pid.0);
    let us = get_time_us();
    let time = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(&time as *const TimeVal as *const u8, size_of::<TimeVal>())
    };
    if copy_to_user(current_user_token(), ts as *mut u8, bytes) {
        0
    } else {
        -1
    }
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel:pid[{}] sys_mmap", current_task().unwrap().pid.0);
    if start % PAGE_SIZE != 0 || len == 0 || prot & !0x7 != 0 {
        return -1;
    }
    let Some(end) = start.checked_add(len) else {
        return -1;
    };
    if start >= 0x8000_0000 || end > 0x8000_0000 {
        return -1;
    }

    let mut permission = MapPermission::empty();
    if prot & 0x1 != 0 {
        permission |= MapPermission::R;
    }
    if prot & 0x2 != 0 {
        permission |= MapPermission::W;
    }
    if prot & 0x4 != 0 {
        permission |= MapPermission::X;
    }
    if permission.is_empty() {
        return -1;
    }
    permission |= MapPermission::U;

    let task = current_task().unwrap();
    if task.mmap(start, len, permission) {
        0
    } else {
        -1
    }
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_munmap", current_task().unwrap().pid.0);
    if start % PAGE_SIZE != 0 || len == 0 || len % PAGE_SIZE != 0 {
        return -1;
    }
    if start.checked_add(len).is_none() {
        return -1;
    }
    if current_task().unwrap().munmap(start, len) {
        0
    } else {
        -1
    }
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(path: *const u8) -> isize {
    let current = current_task().unwrap();
    trace!("kernel:pid[{}] sys_spawn", current.pid.0);
    let token = current_user_token();
    let Some(path) = translated_str(token, path) else {
        return -1;
    };
    let Some(data) = get_app_data_by_name(path.as_str()) else {
        return -1;
    };

    let child = Arc::new(TaskControlBlock::new(data));
    child.inner_exclusive_access().parent = Some(Arc::downgrade(&current));
    current
        .inner_exclusive_access()
        .children
        .push(Arc::clone(&child));
    let pid = child.getpid();
    add_task(child);
    pid as isize
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> isize {
    let task = current_task().unwrap();
    trace!("kernel:pid[{}] sys_set_priority", task.pid.0);
    if prio < 2 {
        return -1;
    }
    let mut inner = task.inner_exclusive_access();
    inner.priority = prio;
    inner.stride = crate::task::priority_to_stride(prio);
    prio
}
