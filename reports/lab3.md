# rCore Lab 3：进程管理与优先级调度

## 1. 实验目的

本实验从 ch4 的静态任务数组过渡到可动态创建、替换、等待和回收的进程系统。通过 TCB、父子进程关系和调度器，理解 `fork`、`exec`、`waitpid`、`spawn` 以及按优先级调度的实现方式。

## 2. 实验环境

- Windows + WSL2 + Ubuntu 20.04
- Rust `nightly-2024-05-02`
- `riscv64gc-unknown-none-elf` 目标
- RustSBI 0.4.1
- QEMU 11.1
- rCore-Tutorial ch5 作业分支

## 3. 进程控制块与进程关系

ch5 的 `TaskControlBlock` 分为不可变部分和可变部分。不可变部分保存 PID 与内核栈；`TaskControlBlockInner` 保存页表、TrapContext、任务上下文、运行状态、退出码、堆边界和父子关系。TCB 使用 `Arc` 共享，父进程通过 `Weak` 引用父节点，从而避免父子引用形成循环。

父进程的 `children` 保存所有直接子进程。子进程退出后状态变为 `Zombie`，退出码暂存在 TCB 中，用户空间和内核栈等资源被回收。若父进程先退出，它仍存活的子进程会重新挂到 `initproc` 下，由初始进程继续等待和回收。

## 4. fork、exec 与 waitpid

`fork` 为子进程复制当前 `MemorySet`，包括代码、数据、用户栈和 TrapContext，并分配新的 PID 与内核栈。父进程返回子进程 PID，子进程的 `a0` 被设置为 0。

`exec` 根据应用名取得新的 ELF，替换当前进程的地址空间、入口地址和 TrapContext，并重置堆底与 program break。它保留调用进程的 PID、父子关系及调度属性。

`waitpid` 只等待当前进程的直接子进程。没有指定子进程时返回 -1；子进程仍在运行则返回 -2；找到僵尸子进程后，将其退出码安全复制到用户地址，从 `children` 中移除并释放该 TCB，最后返回子进程 PID。

## 5. spawn 的实现

`spawn` 不采用 `fork + exec`。内核先安全读取用户态应用名，在静态应用数据中查找对应 ELF，然后直接调用 `TaskControlBlock::new` 创建完整的新进程，设置父进程、加入父进程的子进程列表和就绪队列，并将新 PID 返回给调用者。

这种方式不复制父进程地址空间，因此创建开销更小，也不会继承父进程当前的用户寄存器状态。`ch5_spawn0` 连续创建 40 个 `ch5_getpid` 子进程；`ch5_spawn1` 通过三个不同退出码的程序验证 `wait` 和 `waitpid` 能准确取得 PID 与退出码。

## 6. 优先级与 stride 调度

每个进程维护 `priority`、`stride` 和 `pass`。优先级合法范围为 `priority >= 2`，非法值返回 -1。步长按下式计算：

```text
stride = BIG_STRIDE / priority
```

调度器从就绪队列中选择 `pass` 最小的进程；进程获得一次执行机会后执行 `pass += stride`。因此优先级越高，步长越小，在相同时间内能获得更多 CPU 时间，同时低优先级进程仍然可以得到执行。

最初尝试使用严格按优先级排序的队列，但高优先级进程会长期压住其他进程，使 stride 测例运行超过 30 秒。改用 stride 调度后，所有进程持续推进。测试中优先级 5 到 10 的计数与优先级近似成正比，且整体运行能够在超时限制内结束。

## 7. ch4 功能的回归

ch5 仍包含前一章的地址空间测例，因此重新实现了 `get_time`、`mmap` 和 `munmap`。用户内存复制统一检查低半区地址、加法溢出、页表有效性和读写权限，并支持跨页缓冲区。`mmap` 要求起始地址页对齐，长度会向上取整；`munmap` 只释放与完整映射区域一致的页对齐区间。

## 8. 测试结果

执行：

```sh
make -C ci-user test CHAPTER=5
```

输出包含：

```text
Test spawn0 OK!
Test wait OK!
Test waitpid OK!
Test set_priority OK!
```

官方检查器结果：

```text
Test passed: 15/15
```

Stride 测例也成功输出六组计数，程序最终输出 `ch5 Usertests passed!`。

## 9. 实验总结

本实验中，进程从静态任务变成了有 PID、父子关系和退出状态的动态实体。`spawn` 需要直接建立新地址空间，`waitpid` 则必须区分“没有子进程”“子进程未退出”和“子进程已是僵尸”三种情况。调度部分让我认识到，仅按优先级排序会造成饿死，stride 调度通过累计虚拟运行时间，在保证优先级差异的同时维持系统整体前进。最终全部 15 项官方检查通过。

## 10. AI 工具使用说明

使用 OpenAI Codex 辅助分析 ch5 的进程管理器、定位优先级队列导致的超时，并整理报告。源码通过目标平台编译和官方检查器验证，未修改测试程序或检查器判定逻辑。
