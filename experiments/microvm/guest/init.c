/* SPDX-License-Identifier: Apache-2.0
 * Experimental PID 1 and a bounded, non-shell administrative control protocol.
 * ENABLE_KEEPER adds the actual devcroft keeper as a separately confined child.
 * No host paths or command strings are accepted from the wire.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/fs.h>
#include <linux/landlock.h>
#include <linux/vm_sockets.h>
#include <net/if.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mount.h>
#include <sys/prctl.h>
#include <sys/reboot.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

static void fail(const char *what) {
    dprintf(2, "guest: %s: %s\n", what, strerror(errno));
    sync();
    reboot(RB_POWER_OFF);
    for (;;) pause();
}

static void reap(int unused) {
    (void)unused;
    while (waitpid(-1, NULL, WNOHANG) > 0) {}
}

static void mount_required(const char *type, const char *target) {
    if (mount(type, target, type, MS_NOSUID | MS_NODEV, NULL) < 0) fail(target);
}

static int send_all(int fd, const char *data) {
    size_t len = strlen(data);
    while (len) {
        ssize_t n = send(fd, data, len, MSG_NOSIGNAL);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) return -1;
        data += n;
        len -= (size_t)n;
    }
    return 0;
}

#ifdef ENABLE_ADAPTABLE
/* Fixed argv only. Wire commands cannot choose paths, packages or a shell. */
static int run_fixed(char *const argv[], unsigned int seconds) {
    pid_t child = fork();
    if (child < 0) fail("administrative fork");
    if (!child) {
        if (setsid() < 0) _exit(126);
        execv(argv[0], argv);
        _exit(127);
    }
    int status;
    for (unsigned int i = 0; i < seconds * 100; ++i) {
        pid_t done = waitpid(child, &status, WNOHANG);
        if (done == child) return WIFEXITED(status) && WEXITSTATUS(status) == 0;
        if (done < 0 && errno != EINTR) fail("administrative wait");
        usleep(10000);
    }
    (void)kill(-child, SIGKILL);
    while (waitpid(child, &status, 0) < 0 && errno == EINTR) {}
    return 0;
}

static void write_marker(const char *path, const char *value) {
    int fd = open(path, O_CREAT | O_EXCL | O_WRONLY | O_CLOEXEC, 0600);
    size_t len = strlen(value);
    if (fd < 0 || write(fd, value, len) != (ssize_t)len || fsync(fd)) fail("setup marker");
    close(fd);
}

#ifdef ENABLE_SETUP_FAULT_PROBE
/* Disposable interruption fixture only. Each named point stalls PID 1 exactly
 * once per private disk, so the host can kill the helper there and the next
 * boot proceeds to the following point. Absent from ordinary guest binaries. */
static void fault_point(const char *name) {
    char path[96];
    snprintf(path, sizeof(path), "/private-base/.test-fault-%s", name);
    if (!access(path, F_OK)) return;
    write_marker(path, name);
    sync();
    struct statvfs space;
    if (statvfs("/private-base", &space)) fail("setup storage sample");
    dprintf(2, "guest: test-only setup storage point=%s block_size=%llu blocks=%llu free=%llu available=%llu\n",
            name, (unsigned long long)space.f_frsize, (unsigned long long)space.f_blocks,
            (unsigned long long)space.f_bfree, (unsigned long long)space.f_bavail);
    dprintf(2, "guest: test-only setup stall point=%s\n", name);
    for (;;) pause();
}
#else
#define fault_point(name) ((void)0)
#endif

/* Staging trees are never published state. A pending baseline was never
 * renamed into place; rootfs.next is either an unexchanged partial recipe or
 * the superseded tree after the exchange. Removing either is always safe. */
static void discard_staging(const char *path) {
    struct stat st;
    if (lstat(path, &st)) { if (errno == ENOENT) return; fail("inspect staging tree"); }
    char *argv[] = {"/bin/busybox", "rm", "-rf", (char *)path, NULL};
    if (!run_fixed(argv, 120)) fail("discard staging tree");
    sync();
    dprintf(2, "guest: discarded interrupted %s\n", path);
}

static void prepare_private_base(void) {
    if (mkdir("/private-base/workspace", 0700) && errno != EEXIST) fail("private workspace");
    discard_staging("/private-base/rootfs.pending");
    discard_staging("/private-base/rootfs.next");
    if (access("/private-base/rootfs/.baseline-ready", F_OK)) {
        if (mkdir("/private-base/rootfs.pending", 0700)) fail("pending base exists");
        char *argv[] = {"/bin/busybox", "cp", "-a", "/toolchain/usr", "/toolchain/lib",
            "/toolchain/etc", "/toolchain/bin", "/toolchain/sbin", "/toolchain/var",
            "/private-base/rootfs.pending/", NULL};
        if (!run_fixed(argv, 120)) fail("copy private baseline");
        fault_point("baseline-copy");
        write_marker("/private-base/rootfs.pending/.baseline-ready", "baseline-v1\n");
        if (rename("/private-base/rootfs.pending", "/private-base/rootfs")) fail("publish baseline");
        sync();
    }
    if (mount("/private-base/workspace", "/workspace", NULL, MS_BIND, NULL)) fail("bind private workspace");
}

static int recipe_applied(void) {
    char actual[65] = {0};
    int fd = open("/private-base/rootfs/.recipe-applied", O_RDONLY | O_CLOEXEC);
    if (fd < 0 && errno == ENOENT) return 0;
    if (fd < 0 || read(fd, actual, sizeof(actual)) != 64) fail("read recipe marker");
    close(fd);
    if (strcmp(actual, SETUP_RECIPE_HASH)) fail("private base recipe mismatch");
    return 1;
}

#ifdef ENABLE_SETUP_WINDOW
/* Apply the recipe to a copy and publish it with one atomic exchange, so an
 * interruption at any point leaves exactly the baseline or exactly the applied
 * base, never a package manager's partial write. The published tree's own apk
 * runs against the copy; it is never modified in place. */
static int install_recipe(void) {
    char *check[] = {"/bin/busybox", "sha256sum", "-c", "/toolchain/adaptable/SHA256SUMS", NULL};
    char *stage[] = {"/bin/busybox", "cp", "-a", "/private-base/rootfs", "/private-base/rootfs.next", NULL};
    char *install[] = {"/private-base/rootfs/sbin/apk", "--root", "/private-base/rootfs.next",
        "--no-network", "--no-scripts", "--force-non-repository", "--keys-dir",
        "/toolchain/etc/apk/keys", "add", SETUP_PACKAGES, NULL};
    discard_staging("/private-base/rootfs.next");
    if (!run_fixed(check, 30)) return 0;
    if (!run_fixed(stage, 120)) { discard_staging("/private-base/rootfs.next"); return 0; }
    fault_point("recipe-staged");
    if (!run_fixed(install, 30)) { discard_staging("/private-base/rootfs.next"); return 0; }
    fault_point("recipe-installed");
    write_marker("/private-base/rootfs.next/.recipe-applied", SETUP_RECIPE_HASH);
    sync();
    fault_point("recipe-marked");
    if (syscall(SYS_renameat2, AT_FDCWD, "/private-base/rootfs.next",
                AT_FDCWD, "/private-base/rootfs", RENAME_EXCHANGE)) fail("publish recipe");
    sync();
    fault_point("recipe-exchanged");
    discard_staging("/private-base/rootfs.next");
    return 1;
}
#endif
#endif

static int readonly_probe(void) {
    int fd = open("/dev/vda", O_RDWR | O_CLOEXEC);
    if (fd < 0) return -1; /* A missing/broken disk is not a denial pass. */
    unsigned char before[512], after[512], attempt[512];
    if (pread(fd, before, sizeof(before), 0) != sizeof(before)) fail("read runtime");
    if (memcmp(before, "DEVCROFT_RUNTIME_V1", 19)) fail("runtime identity");
    memcpy(attempt, before, sizeof(attempt));
    attempt[64] ^= 0x5a;
    int writable = 0;
    /* Drop the guest kernel's RO flag: the VMM must still reject the write. */
    (void)ioctl(fd, BLKROSET, &writable);
    ssize_t written = pwrite(fd, attempt, sizeof(attempt), 0);
    if (pread(fd, after, sizeof(after), 0) != sizeof(after)) fail("reread runtime");
    int denied = written < 0 && !memcmp(before, after, sizeof(before));
    close(fd);
    return denied;
}

static unsigned int update_state(void) {
    unsigned char sector[512] = {0};
    int fd = open("/dev/vdb", O_RDWR | O_CLOEXEC);
    if (fd < 0) fail("open state");
    if (pread(fd, sector, sizeof(sector), 4096) != sizeof(sector)) fail("read state");
    unsigned int count = 0;
    if (sector[0] && sscanf((char *)sector, "DEVCROFT_STATE_V1 %u", &count) != 1)
        fail("state identity");
    if (count > 1000000) fail("state counter out of range");
    memset(sector, 0, sizeof(sector));
    snprintf((char *)sector, sizeof(sector), "DEVCROFT_STATE_V1 %u", ++count);
    if (pwrite(fd, sector, sizeof(sector), 4096) != sizeof(sector) || fsync(fd) < 0)
        fail("persist state");
    close(fd);
    return count;
}

static int landlock_probe(int *abi) {
    *abi = (int)syscall(SYS_landlock_create_ruleset, NULL, 0,
                       LANDLOCK_CREATE_RULESET_VERSION);
    if (*abi < 1) { *abi = 0; return 0; }
    if (mkdir("/tmp/policy-allowed", 0700) || mkdir("/tmp/policy-denied", 0700))
        fail("policy directories");
    const char *paths[] = {"/tmp/policy-allowed/file", "/tmp/policy-denied/file"};
    for (unsigned int i = 0; i < 2; ++i) {
        int fd = open(paths[i], O_CREAT | O_RDWR | O_CLOEXEC, 0600);
        if (fd < 0 || write(fd, "sentinel", 8) != 8) fail("policy baseline");
        close(fd);
    }
    /* Probe in a child: PID 1 must retain authority to serve control and stop. */
    pid_t child = fork();
    if (child < 0) fail("policy fork");
    if (!child) {
        uint64_t rights = LANDLOCK_ACCESS_FS_READ_FILE | LANDLOCK_ACCESS_FS_WRITE_FILE |
                          LANDLOCK_ACCESS_FS_READ_DIR;
        struct landlock_ruleset_attr ruleset = { .handled_access_fs = rights };
        int fd = (int)syscall(SYS_landlock_create_ruleset, &ruleset, sizeof(ruleset), 0);
        int path = open("/tmp/policy-allowed", O_PATH | O_CLOEXEC);
        struct landlock_path_beneath_attr rule = { .allowed_access = rights, .parent_fd = path };
        if (fd < 0 || path < 0 ||
            syscall(SYS_landlock_add_rule, fd, LANDLOCK_RULE_PATH_BENEATH, &rule, 0) ||
            prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) ||
            syscall(SYS_landlock_restrict_self, fd, 0)) _exit(1);
        close(path); close(fd);
        int positive = open(paths[0], O_RDWR | O_CLOEXEC);
        if (positive < 0 || write(positive, "allowed", 7) != 7) _exit(2);
        close(positive);
        errno = 0;
        int negative = open(paths[1], O_RDONLY | O_CLOEXEC);
        if (negative >= 0 || errno != EACCES) _exit(3);
        errno = 0;
        negative = open(paths[1], O_WRONLY | O_CLOEXEC);
        if (negative >= 0 || errno != EACCES) _exit(4);
        _exit(0);
    }
    int status;
    if (waitpid(child, &status, 0) != child) fail("policy wait");
    return WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

#ifdef ENABLE_KEEPER
static void start_keeper(void) {
#ifdef ENABLE_AGENT_PROBE
    /* Fixture-only DNS aliases preserve the real TLS server name while the
     * trusted SSH client forwards this single loopback port to vendor:443.
     * No NIC, resolver, filesystem share or general proxy is introduced. */
    if (mount("/agent-api-hosts", "/etc/hosts", NULL, MS_BIND, NULL) ||
        mount(NULL, "/etc/hosts", NULL, MS_BIND | MS_REMOUNT | MS_RDONLY |
              MS_NOSUID | MS_NODEV | MS_NOEXEC, NULL)) fail("agent hosts bind");
#endif
    /* Only loopback exists: enabling it adds no guest/host NIC or egress path. */
    int net = socket(AF_INET, SOCK_DGRAM | SOCK_CLOEXEC, 0);
    struct ifreq loopback = {0};
    strcpy(loopback.ifr_name, "lo");
    if (net < 0 || ioctl(net, SIOCGIFFLAGS, &loopback)) fail("loopback flags");
    loopback.ifr_flags |= IFF_UP;
    if (ioctl(net, SIOCSIFFLAGS, &loopback)) fail("enable loopback");
    close(net);
    if (mkdir("/dev/pts", 0755) && errno != EEXIST) fail("mkdir devpts");
    if (mount("devpts", "/dev/pts", "devpts", MS_NOSUID | MS_NOEXEC,
              "mode=0620,ptmxmode=0666") < 0) fail("mount devpts");
    if (mkdir("/run/keeper", 0700)) fail("keeper directory");
    pid_t child = fork();
    if (child < 0) fail("fork keeper");
    if (!child) {
        char *argv[] = {"/guest-keeper", NULL};
        char *env[] = {"PATH=/usr/bin:/bin", "HOME=/home/devcroft", "SHELL=/bin/sh",
                      "DEVCROFT_TASK=fixture-task", "DEVCROFT_SHELL=/bin/sh",
                      "DEVCROFT_SSH_HOST_KEY=fixture-not-a-key", NULL};
        execve(argv[0], argv, env);
        _exit(127);
    }
    for (unsigned int i = 0; i < 100; ++i) {
        int status;
        if (waitpid(child, &status, WNOHANG) == child) fail("keeper exited before READY");
        if (!access("/tmp/keeper-ready", F_OK)) return;
        usleep(100000);
    }
    fail("keeper readiness timeout");
}
#endif

int main(void) {
    signal(SIGPIPE, SIG_IGN);
    mount_required("proc", "/proc");
    mount_required("sysfs", "/sys");
    if (mount("devtmpfs", "/dev", "devtmpfs", MS_NOSUID, NULL) < 0) fail("devtmpfs");
    mount_required("tmpfs", "/run");
    mount_required("tmpfs", "/tmp");
#ifdef ENABLE_WORKLOAD
    if (mount("/dev/vdc", "/toolchain", "ext4", MS_RDONLY | MS_NOSUID | MS_NODEV, NULL))
        fail("mount immutable toolchain");
#ifdef ENABLE_ADAPTABLE
    if (mount("/dev/vdd", "/private-base", "ext4", MS_NOSUID | MS_NODEV, NULL))
        fail("mount private base");
    prepare_private_base();
#else
    if (mount("/dev/vdd", "/workspace", "ext4", MS_NOSUID | MS_NODEV, NULL))
        fail("mount private workspace");
#endif
#endif
    int landlock_abi = 0;
    int policy = landlock_probe(&landlock_abi);
#ifdef ENABLE_ADAPTABLE
    int applied = recipe_applied();
#ifdef ENABLE_SETUP_WINDOW
    int setup_open = 1;
#else
    int setup_open = 0;
#endif
#endif
#if defined(ENABLE_KEEPER) && !defined(ENABLE_SETUP_WINDOW)
    start_keeper();
#endif
#ifndef ENABLE_SETUP_WINDOW
    signal(SIGCHLD, reap);
#endif
    int ro = readonly_probe();
    if (ro != 1) fail("runtime backend did not enforce read-only");
    unsigned int state = update_state();
    int server = socket(AF_VSOCK, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (server < 0) fail("vsock socket");
    struct sockaddr_vm addr = { .svm_family = AF_VSOCK,
        .svm_cid = VMADDR_CID_ANY, .svm_port = 1024 };
    if (bind(server, (struct sockaddr *)&addr, sizeof(addr)) < 0 || listen(server, 8) < 0)
        fail("vsock listen");
    dprintf(1, "guest: ready state=%u ro=%d\n", state, ro);
#ifdef ENABLE_SETUP_WINDOW
    dprintf(1, "guest: setup open recipe=%s\n", SETUP_RECIPE_HASH);
#endif
    for (;;) {
        int fd = accept4(server, NULL, NULL, SOCK_CLOEXEC);
        if (fd < 0) { if (errno == EINTR) continue; fail("accept"); }
        struct timeval timeout = { .tv_sec = 10 };
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
        char greeting[96];
#ifdef ENABLE_ADAPTABLE
        snprintf(greeting, sizeof(greeting), "READY 1 state=%u ro=%d landlock=%d policy=%d setup=%s\n",
                 state, ro, landlock_abi, policy, setup_open ? "open" : "closed");
#else
        snprintf(greeting, sizeof(greeting), "READY 1 state=%u ro=%d landlock=%d policy=%d\n",
                 state, ro, landlock_abi, policy);
#endif
        if (send_all(fd, greeting) < 0) { close(fd); continue; }
        char line[1024];
        size_t used = 0;
        for (;;) {
            char c;
            ssize_t n = recv(fd, &c, 1, 0);
            if (n < 0 && errno == EINTR) continue;
            if (n == 0) { send_all(fd, "EOF\n"); break; }
            if (n < 0) break;
            if (c != '\n') {
                if (used + 1 >= sizeof(line) || c < 32 || c > 126) {
                    send_all(fd, "ERR frame\n"); break;
                }
                line[used++] = c;
                continue;
            }
            line[used] = 0;
#ifdef ENABLE_ADAPTABLE
            if (!strncmp(line, "SETUP ", 6)) {
                if (!setup_open) { send_all(fd, "ERR setup closed\n"); }
                else if (!strcmp(line, "SETUP CLOSE")) {
                    if (!applied) { send_all(fd, "ERR setup unapplied\n"); }
                    else {
                        start_keeper();
                        signal(SIGCHLD, reap);
                        setup_open = 0;
                        send_all(fd, "OK setup closed\n");
                    }
                } else if (!strcmp(line, "SETUP APPLY " SETUP_RECIPE_HASH)) {
#ifdef ENABLE_SETUP_WINDOW
                    if (applied) send_all(fd, "ERR setup applied\n");
                    else if (!install_recipe()) send_all(fd, "ERR setup install\n");
                    else { applied = 1; send_all(fd, "OK setup applied\n"); }
#else
                    send_all(fd, "ERR setup closed\n");
#endif
                } else { send_all(fd, "ERR setup recipe\n"); }
            } else
#endif
            if (!strncmp(line, "PING ", 5)) {
                send_all(fd, "PONG "); send_all(fd, line + 5); send_all(fd, "\n");
#ifdef ENABLE_PID1_EXIT_PROBE
            } else if (!strcmp(line, "TEST PID1 EXIT 86")) {
                /* Disposable fault fixture only. No sync or orderly shutdown:
                 * exiting global PID 1 must cause the kernel's init-death panic.
                 * This entire branch is absent from ordinary guest binaries. */
                dprintf(2, "guest: test-only PID1 exit=86\n");
                _exit(86);
#endif
            } else if (!strcmp(line, "SHUTDOWN")) {
                send_all(fd, "BYE\n");
                close(fd);
                sync();
                reboot(RB_POWER_OFF);
                fail("poweroff returned");
            } else {
                send_all(fd, "ERR command\n");
            }
            used = 0;
        }
        close(fd);
    }
}
