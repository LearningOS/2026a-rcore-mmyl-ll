# rCore Lab 1：多道程序与系统调用 trace

## 1. 实验目的

本实验在 rCore-Tutorial ch3 多道程序和分时调度系统的基础上，实现用于读取、写入用户内存以及查询当前任务系统调用次数的 `trace` 系统调用。通过该实验理解 TCB、Trap 分发、系统调用编号和参数传递，以及每个任务独立维护运行信息的方法。

## 2. 实验环境

- Windows + WSL2 + Ubuntu 20.04
- Rust `nightly-2024-05-02`
- `riscv64gc-unknown-none-elf` 目标
- `cargo-binutils 0.4.0`
- RustSBI 0.4.1
- QEMU 11.1
- rCore-Tutorial ch3 作业分支

## 3. trace 系统调用设计

`trace` 的系统调用号为 410，接收 `trace_request`、`id` 和 `data` 三个参数。

### 3.1 读取用户内存

当 `trace_request == 0` 时，将 `id` 解释为用户地址，读取该地址处的一个字节并作为返回值。此时 `data` 不参与操作。

### 3.2 写入用户内存

当 `trace_request == 1` 时，将 `id` 解释为用户地址，把 `data` 转换为 `u8` 后写入该地址，并返回 0。

### 3.3 查询系统调用次数

当 `trace_request == 2` 时，以 `id` 作为被查询的系统调用号，返回当前任务对应的调用次数。本次 `trace` 调用本身也必须在查询前计入，因此测试第一次查询 trace 时得到 2。

其他 `trace_request` 返回 -1。

当前 ch3 尚未建立独立地址空间和页表，因此用户程序与内核共享物理地址空间。按照本章实验要求，`trace` 暂不对用户地址进行权限或合法性翻译，后续 ch4 会引入用户指针检查和地址空间隔离。

## 4. 系统调用计数的实现

系统调用计数不直接放入 TCB。ch3 的 TCB 数组在内核栈上完成初始化，若为 16 个任务分别增加 512 个 `usize`，会显著扩大临时数组并引发内核栈溢出风险。

因此本实验使用二维全局计数表：

```text
SYSCALL_COUNTS[task_id][syscall_id]
```

每个元素使用 `AtomicUsize`，任务编号使用 `TaskManager` 当前任务的索引。该设计既保留了按任务独立统计的语义，又不改变 TCB 大小。

在 `trap_handler()` 处理 `UserEnvCall` 时，先从 `a7` 寄存器取得系统调用号，并在分发到具体系统调用之前累加计数。这样处理 `trace_request == 2` 时，本次 trace 调用已经被统计。

## 5. 修改点

1. `os/src/task/mod.rs`：增加按任务编号和系统调用编号索引的全局计数表，并提供累加和查询函数。
2. `os/src/trap/mod.rs`：在每次 U-mode `ecall` 分发前记录当前系统调用号。
3. `os/src/syscall/process.rs`：实现 trace 的读取、写入和计数查询三个分支。
4. `os/src/task/task.rs`：保持 TCB 结构不变，避免因计数数组扩大 TCB 而占用过多内核栈。

## 6. 测试结果

使用官方检查器执行：

```sh
make -C ci-user test CHAPTER=3
```

最终运行输出包含：

```text
string from task trace test

Test trace OK!
Test sleep OK!
```

检查器的 7 个匹配项全部通过：

```text
[PASS] get_time OK! (...)
[PASS] Test sleep OK!
[PASS] current time_msec = (...)
[PASS] time_msec = (...) after sleeping (...) ticks, delta = (...)ms!
[PASS] Test sleep1 passed!
[PASS] string from task trace test
[PASS] Test trace OK!

Test passed: 7/7
```

## 7. 遇到的问题与解决方法

最初将计数数组直接放入 TCB，内核虽然能够编译，但检查器运行出现超时和不稳定。分析后发现扩大的 TCB 会通过内核栈上的临时数组初始化，可能造成栈空间不足。改成任务编号索引的全局原子计数表后，TCB 大小保持不变，测试稳定达到 7/7。

此外，WSL 无法直接访问 GitHub 上的 Git 依赖，因此使用 Windows Git 下载 `rcore-os/riscv` 的本地镜像，并让 Cargo 通过 Git CLI 使用该镜像。Ubuntu 20.04 自带 QEMU 版本过低，本实验通过本机 QEMU 11.1 运行，以满足课程要求的 QEMU 8.2 以上版本。

## 8. 实验总结

通过本实验，我进一步理解了系统调用从用户态 `ecall`、Trap 保存上下文、读取系统调用号和参数，到内核分发和执行的具体过程。系统调用计数必须发生在分发之前，才能正确包含当前调用。同时，实现应服从系统的内存布局限制，不能只追求数据结构上的直观性。最终 trace 的读写和查询功能均通过官方 7 项检查，也为后续地址空间章节的用户指针翻译和权限检查打下了基础。

## 9. AI 工具使用说明

使用 OpenAI Codex 辅助分析 ch3 源码、定位 trace 缺口、讨论 TCB 扩展导致的内核栈问题以及整理实验报告。所有源码修改、构建、官方检查器测试和结果验证均在本地完成；提交前重新运行了官方 ch3 检查。
