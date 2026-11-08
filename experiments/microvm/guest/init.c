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
    if (mkdir("/dev/pts", 0755) && errno != EEXIST) fail("mkdir devpts");
    if (mount("devpts", "/dev/pts", "devpts", MS_NOSUID | MS_NOEXEC,
              "mode=0620,ptmxmode=0666") < 0) fail("mount devpts");
    if (mkdir("/run/keeper", 0700)) fail("keeper directory");
    pid_t child = fork();
    if (child < 0) fail("fork keeper");
    if (!child) {
        char *argv[] = {"/guest-keeper", NULL};
        char *env[] = {"PATH=/bin", "HOME=/home/devcroft", "SHELL=/bin/sh",
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
    int landlock_abi = 0;
    int policy = landlock_probe(&landlock_abi);
#ifdef ENABLE_KEEPER
    start_keeper();
#endif
    signal(SIGCHLD, reap);
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
    for (;;) {
        int fd = accept4(server, NULL, NULL, SOCK_CLOEXEC);
        if (fd < 0) { if (errno == EINTR) continue; fail("accept"); }
        struct timeval timeout = { .tv_sec = 10 };
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
        char greeting[96];
        snprintf(greeting, sizeof(greeting), "READY 1 state=%u ro=%d landlock=%d policy=%d\n",
                 state, ro, landlock_abi, policy);
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
            if (!strncmp(line, "PING ", 5)) {
                send_all(fd, "PONG "); send_all(fd, line + 5); send_all(fd, "\n");
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
