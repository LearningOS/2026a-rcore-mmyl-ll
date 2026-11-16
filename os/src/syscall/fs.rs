//! File and filesystem-related syscalls
use crate::fs::{open_file, OpenFlags, Stat, ROOT_INODE};
use crate::mm::{
    copy_to_user, translated_byte_buffer, translated_byte_buffer_mut, translated_str, UserBuffer,
};
use crate::task::{current_task, current_user_token};
use core::mem::size_of;

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        let Some(buffers) = translated_byte_buffer(token, buf, len) else {
            return -1;
        };
        file.write(UserBuffer::new(buffers)) as isize
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        let Some(buffers) = translated_byte_buffer_mut(token, buf, len) else {
            return -1;
        };
        file.read(UserBuffer::new(buffers)) as isize
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
    let Some(path) = translated_str(token, path) else {
        return -1;
    };
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}

/// YOUR JOB: Implement fstat.
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    let task = current_task().unwrap();
    trace!("kernel:pid[{}] sys_fstat", task.pid.0);
    let token = task.get_user_token();
    let inner = task.inner_exclusive_access();
    let Some(file) = inner.fd_table.get(fd).and_then(|file| file.clone()) else {
        return -1;
    };
    drop(inner);
    let stat = file.stat();
    let bytes = unsafe {
        core::slice::from_raw_parts(&stat as *const Stat as *const u8, size_of::<Stat>())
    };
    if copy_to_user(token, st as *mut u8, bytes) {
        0
    } else {
        -1
    }
}

/// YOUR JOB: Implement linkat.
pub fn sys_linkat(old_name: *const u8, new_name: *const u8) -> isize {
    let task = current_task().unwrap();
    trace!("kernel:pid[{}] sys_linkat", task.pid.0);
    let token = task.get_user_token();
    let Some(old_name) = translated_str(token, old_name) else {
        return -1;
    };
    let Some(new_name) = translated_str(token, new_name) else {
        return -1;
    };
    if ROOT_INODE.link(old_name.as_str(), new_name.as_str()) {
        0
    } else {
        -1
    }
}

/// YOUR JOB: Implement unlinkat.
pub fn sys_unlinkat(name: *const u8) -> isize {
    let task = current_task().unwrap();
    trace!("kernel:pid[{}] sys_unlinkat", task.pid.0);
    let token = task.get_user_token();
    let Some(name) = translated_str(token, name) else {
        return -1;
    };
    if ROOT_INODE.unlink(name.as_str()).is_some() {
        0
    } else {
        -1
    }
}
