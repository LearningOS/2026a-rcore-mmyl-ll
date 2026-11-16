use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task, ProcessControlBlock};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;

const DEADLOCK: isize = -0xdead;

fn current_tid() -> usize {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid
}

fn semaphore_request_would_deadlock(
    process: &Arc<ProcessControlBlock>,
    task: usize,
    _sem_id: usize,
) -> bool {
    let (capacities, mut holders, requests) = {
        let process_inner = process.inner_exclusive_access();
        (
            process_inner.semaphore_capacities.clone(),
            process_inner.semaphore_holders.clone(),
            process_inner.semaphore_requests.clone(),
        )
    };
    let mut available = alloc::vec::Vec::with_capacity(capacities.len());
    for (sem_id, capacity) in capacities.iter().enumerate() {
        let used = holders
            .get(sem_id)
            .map(|holders| holders.values().sum::<usize>())
            .unwrap_or(0);
        available.push(capacity.saturating_sub(used));
    }

    let mut tasks = alloc::collections::BTreeMap::new();
    for task_id in requests.keys() {
        tasks.insert(*task_id, ());
    }
    for holders_for_sem in holders.iter() {
        for task_id in holders_for_sem.keys() {
            tasks.insert(*task_id, ());
        }
    }

    let mut finished = alloc::collections::BTreeMap::new();
    loop {
        let mut progress = false;
        for task_id in tasks.keys() {
            if finished.contains_key(task_id) {
                continue;
            }
            let can_finish = match requests.get(task_id) {
                None => true,
                Some(requested) => available.get(*requested).copied().unwrap_or(0) > 0,
            };
            if !can_finish {
                continue;
            }
            if let Some(requested) = requests.get(task_id) {
                available[*requested] -= 1;
            }
            for (sem_index, holders_for_sem) in holders.iter_mut().enumerate() {
                if let Some(count) = holders_for_sem.remove(task_id) {
                    available[sem_index] += count;
                }
            }
            finished.insert(*task_id, ());
            progress = true;
        }
        if !progress || finished.contains_key(&task) {
            break;
        }
    }
    !finished.contains_key(&task)
}

fn mutex_wait_reaches(
    process: &Arc<ProcessControlBlock>,
    task: usize,
    target: usize,
    visited: &mut alloc::vec::Vec<usize>,
) -> bool {
    if task == target {
        return true;
    }
    if visited.contains(&task) {
        return false;
    }
    visited.push(task);
    let process_inner = process.inner_exclusive_access();
    let Some(mutex_id) = process_inner.mutex_requests.get(&task).copied() else {
        return false;
    };
    let Some(owner) = process_inner.mutex_owners.get(&mutex_id).copied() else {
        return false;
    };
    drop(process_inner);
    mutex_wait_reaches(process, owner, target, visited)
}

fn mutex_request_would_deadlock(
    process: &Arc<ProcessControlBlock>,
    task: usize,
    mutex_id: usize,
) -> bool {
    let owner = process
        .inner_exclusive_access()
        .mutex_owners
        .get(&mutex_id)
        .copied();
    owner
        .map(|owner| mutex_wait_reaches(process, owner, task, &mut alloc::vec::Vec::new()))
        .unwrap_or(false)
}
/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    let tid = current_tid();
    let owner = process
        .inner_exclusive_access()
        .mutex_owners
        .get(&mutex_id)
        .copied();
    if owner == Some(tid) {
        return DEADLOCK;
    }
    if owner.is_some() {
        let mut process_inner = process.inner_exclusive_access();
        let detect = process_inner.deadlock_detect;
        process_inner.mutex_requests.insert(tid, mutex_id);
        drop(process_inner);
        if detect && mutex_request_would_deadlock(&process, tid, mutex_id) {
            process.inner_exclusive_access().mutex_requests.remove(&tid);
            return DEADLOCK;
        }
    }
    mutex.lock();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.mutex_requests.remove(&tid);
    process_inner.mutex_owners.insert(mutex_id, tid);
    drop(process_inner);
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    mutex.unlock();
    process
        .inner_exclusive_access()
        .mutex_owners
        .remove(&mutex_id);
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_list.len() - 1
    };
    if process_inner.semaphore_capacities.len() <= id {
        process_inner.semaphore_capacities.resize(id + 1, 0);
        process_inner
            .semaphore_holders
            .resize_with(id + 1, alloc::collections::BTreeMap::new);
    }
    process_inner.semaphore_capacities[id] = res_count;
    process_inner.semaphore_holders[id].clear();
    process_inner
        .semaphore_requests
        .retain(|_, sem_id| *sem_id != id);
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    let tid = current_tid();
    let mut process_inner = process.inner_exclusive_access();
    if let Some(holders) = process_inner.semaphore_holders.get_mut(sem_id) {
        if let Some(count) = holders.get_mut(&tid) {
            *count -= 1;
            if *count == 0 {
                holders.remove(&tid);
            }
        }
    }
    drop(process_inner);
    sem.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    let tid = current_tid();
    let available = sem.inner.exclusive_access().count > 0;
    if !available {
        let mut process_inner = process.inner_exclusive_access();
        let detect = process_inner.deadlock_detect;
        process_inner.semaphore_requests.insert(tid, sem_id);
        drop(process_inner);
        if detect && semaphore_request_would_deadlock(&process, tid, sem_id) {
            process
                .inner_exclusive_access()
                .semaphore_requests
                .remove(&tid);
            return DEADLOCK;
        }
    }
    sem.down();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.semaphore_requests.remove(&tid);
    if process_inner.semaphore_holders.len() <= sem_id {
        process_inner
            .semaphore_holders
            .resize_with(sem_id + 1, alloc::collections::BTreeMap::new);
    }
    *process_inner.semaphore_holders[sem_id]
        .entry(tid)
        .or_insert(0) += 1;
    drop(process_inner);
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    trace!("kernel: sys_enable_deadlock_detect");
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    process_inner.deadlock_detect = enabled != 0;
    if enabled == 0 {
        process_inner.mutex_requests.clear();
        process_inner.semaphore_requests.clear();
    }
    0
}
