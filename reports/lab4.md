# rCore Lab 4：文件系统与 I/O

## 1. 实验目的

本实验在 ch5 进程系统的基础上接入 virtio 块设备和 EasyFS，为每个进程增加文件描述符表，实现文件的打开、关闭、读写、状态查询、硬链接和删除，并验证进程退出时文件资源能够正确回收。

## 2. 实验环境

- Windows + WSL2 + Ubuntu 20.04
- Rust `nightly-2024-05-02`
- `riscv64gc-unknown-none-elf` 目标
- RustSBI 0.4.1
- QEMU 11.1
- virtio 块设备与 EasyFS

## 3. 块设备与文件系统结构

内核通过 `VirtIOBlock` 封装 virtio 块设备。`BlockDevice` 以 512 字节为单位读写磁盘块，块缓存位于 `easy-fs` 中。EasyFS 的磁盘布局包括 SuperBlock、inode 位图、inode 区、数据位图和文件数据区；`EasyFileSystem` 负责分配 inode 与数据块，`Inode` 提供目录查找、创建、读取和写入操作。

文件系统的根目录在 `ROOT_INODE` 中打开。文件数据通过 `DiskInode` 的直接块和多级间接块定位，因此文件可以跨越单个 512 字节数据块。所有目录项使用 `DirEntry` 描述，其中包含文件名和对应的 inode 编号。

## 4. 文件描述符与 VFS

`TaskControlBlockInner` 增加了 `fd_table`。新进程默认拥有标准输入、标准输出和标准错误三个文件描述符。`fork` 会复制父进程的 fd 表，并使用 `Arc` 共享已经打开的文件对象；不同进程关闭各自的 fd 不会直接破坏其他进程仍在使用的文件。

`File` trait 统一描述标准输入、标准输出和普通文件，提供 `readable`、`writable`、`read`、`write` 与 `stat`。`OSInode` 保存文件读写偏移、读写权限和 EasyFS inode。读写使用 `UserBuffer` 将可能跨页的用户缓冲区拆分为多个切片。

`sys_open` 根据用户路径查找或创建文件，并在当前进程 fd 表中分配最小空闲描述符；`sys_close` 清除对应表项。进程退出时将整个 fd 表清空，从而释放该进程持有的文件引用。

## 5. fstat、硬链接和 unlink

`sys_fstat` 从 fd 表取得文件对象，将 `dev`、inode 编号、文件类型和链接计数复制到用户空间。普通文件的 inode 编号直接用于区分不同文件；链接计数通过扫描根目录中指向同一 inode 的有效目录项得到。

`Inode::link` 在不创建新 inode 的情况下追加目录项，使不同名称指向同一个 inode。`Inode::unlink` 找到名称后，将该目录项改写为空项，并让后续查找忽略它。这样既保持目录数据结构不变，又能立即阻止被删除名称再次打开。

本实验在文件链接计数降为零后保留 inode 和数据块，实验说明允许这一简化。`ch6_file3` 会重复执行创建、写入、关闭和删除，空目录项可被后续创建重复使用，文件系统不会因名称残留而打开旧文件。

## 6. ch5 功能的回归

ch6 仍会运行地址空间和进程管理测例，因此重新移植了 `get_time`、`mmap`、`munmap`、`spawn`、优先级与 stride 调度。用户内存访问统一检查地址上界、加法溢出、页表有效性和读写权限。`spawn` 通过文件系统读取 ELF，创建与原进程隔离的新地址空间，并建立父子关系。

## 7. 测试结果

执行：

```sh
make -C ci-user test CHAPTER=6
```

输出包含：

```text
Test file0 OK!
Test fstat OK!
Test link OK!
Test mass open/unlink OK!
ch6 Usertests passed!
```

官方检查器结果：

```text
Test passed: 31/31
```

基本文件读写、进程 fork/spawn、内存映射、跨页用户缓冲区以及前几章内容均通过回归测试。

## 8. 遇到的问题与解决方法

ch6 首次构建需要下载 `virtio-drivers`，但 WSL 直接访问 GitHub 出现连接和证书错误。后来通过 Windows Git 把依赖克隆到本地缓存，并配置 Git 的 URL 替换，使 Cargo 可以稳定从本地仓库获取依赖。

实现硬链接时，原 EasyFS 只支持新建独立 inode，没有“为已有 inode 增加目录项”的接口。因此增加了 `link`、`unlink` 和 `nlink`。由于目录项删除后不缩短目录大小，采用空目录项标记删除；这不会影响名称查找和后续创建，同时避免修改磁盘格式。

## 9. 实验总结

本实验把文件系统接入进程抽象：路径最终落到 inode，文件对象通过 fd 表被进程持有，读写偏移属于打开的文件实例，而 inode 和链接计数属于文件系统本身。硬链接让我更直观地理解了“文件名”和“文件实体”的区别。最终实现了文件读写、状态查询、link/unlink，并通过 31/31 官方检查。

## 10. AI 工具使用说明

使用 OpenAI Codex 辅助阅读 EasyFS 和 virtio 驱动代码，设计目录项删除与链接计数方案，并定位依赖下载问题。所有修改均通过目标平台编译和官方检查器验证，未修改测试程序或评分判定逻辑。
