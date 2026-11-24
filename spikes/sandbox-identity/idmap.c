/* add-sandbox-identity 0.4: a user namespace where the caller is inside 0
 * (setup) and a subordinate host id is inside 1000 (the workload), written
 * with newuidmap/newgidmap, combined with a mount namespace, a PID namespace
 * with a fresh /proc, a pivoted tmpfs view and Landlock.
 *
 *   idmap <subuid> <subgid> <workspace> <home> <cmd> [args...]
 *
 * The workload runs as 1000 inside. Build: cc -o idmap idmap.c */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <grp.h>
#include <linux/landlock.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#define DIE(what)                                                      \
    do {                                                               \
        fprintf(stderr, "FAIL %s: %s\n", what, strerror(errno));       \
        exit(9);                                                       \
    } while (0)
#define OK(what) fprintf(stderr, "ok   %s\n", what)

static int p2c[2];
static char *ws, *home;

static void bind(const char *src, const char *root, int ro) {
    char t[4096];
    struct stat st;
    if (stat(src, &st))
        return;
    snprintf(t, sizeof t, "%s%s", root, src);
    for (char *p = t + strlen(root) + 1; *p; p++)
        if (*p == '/') {
            *p = 0;
            mkdir(t, 0755);
            *p = '/';
        }
    if (S_ISDIR(st.st_mode))
        mkdir(t, 0755);
    else
        close(open(t, O_CREAT | O_WRONLY, 0644));
    if (mount(src, t, 0, MS_BIND | MS_REC, 0))
        DIE(src);
    if (ro && mount(0, t, 0, MS_REMOUNT | MS_BIND | MS_RDONLY | MS_NOSUID, 0) &&
        mount(0, t, 0,
              MS_REMOUNT | MS_BIND | MS_RDONLY | MS_NOSUID | MS_NODEV | MS_RELATIME, 0))
        fprintf(stderr, "note read-only %s: %s\n", src, strerror(errno));
}

static void landlock(void) {
    struct landlock_ruleset_attr a = {
        .handled_access_fs = LANDLOCK_ACCESS_FS_WRITE_FILE | LANDLOCK_ACCESS_FS_MAKE_REG |
                             LANDLOCK_ACCESS_FS_MAKE_DIR | LANDLOCK_ACCESS_FS_REMOVE_FILE};
    int fd = syscall(SYS_landlock_create_ruleset, &a, sizeof a, 0);
    if (fd < 0)
        DIE("landlock_create_ruleset");
    const char *allow[] = {ws, home, "/tmp", "/dev"};
    for (int i = 0; i < 4; i++) {
        struct landlock_path_beneath_attr r = {.allowed_access = a.handled_access_fs,
                                               .parent_fd = open(allow[i], O_PATH | O_CLOEXEC)};
        if (r.parent_fd < 0)
            DIE(allow[i]);
        if (syscall(SYS_landlock_add_rule, fd, LANDLOCK_RULE_PATH_BENEATH, &r, 0))
            DIE("landlock_add_rule");
    }
    if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))
        DIE("no_new_privs");
    if (syscall(SYS_landlock_restrict_self, fd, 0))
        DIE("landlock_restrict_self");
}

static int child(char **argv) {
    char c, t[4096], root[] = "/tmp/idmap-root-XXXXXX";
    close(p2c[1]);
    if (read(p2c[0], &c, 1) != 1)
        exit(8);
    if (setresgid(0, 0, 0) || setresuid(0, 0, 0))
        DIE("become setup root");
    if (mount(0, "/", 0, MS_REC | MS_PRIVATE, 0))
        DIE("private propagation");
    if (!mkdtemp(root))
        DIE("mkdtemp");
    if (mount("tmpfs", root, "tmpfs", 0, "mode=0755"))
        DIE("tmpfs root");
    snprintf(t, sizeof t, "%s/tmp", root);
    mkdir(t, 0755);
    if (mount("tmpfs", t, "tmpfs", 0, "mode=1777"))
        DIE("tmpfs /tmp");
    const char *ro[] = {"/nix/store", "/usr", "/etc", 0};
    for (int i = 0; ro[i]; i++)
        bind(ro[i], root, 1);
    const char *dev[] = {"/dev/null", "/dev/urandom", "/dev/tty", 0};
    for (int i = 0; dev[i]; i++)
        bind(dev[i], root, 0);
    const char *ln[] = {"bin", "lib", "lib64", "sbin", 0};
    for (int i = 0; ln[i]; i++) {
        char to[64];
        snprintf(to, sizeof to, "usr/%s", ln[i]);
        snprintf(t, sizeof t, "%s/%s", root, ln[i]);
        if (symlink(to, t))
            DIE("symlink");
    }
    bind(ws, root, 0);
    bind(home, root, 0);
    snprintf(t, sizeof t, "%s/proc", root);
    mkdir(t, 0755);
    if (mount("proc", t, "proc", MS_NOSUID | MS_NODEV | MS_NOEXEC, 0))
        DIE("fresh /proc");
    OK("fresh /proc in the PID namespace");
    snprintf(t, sizeof t, "%s%s", root, home);
    if (chown(t, 1000, 1000))
        fprintf(stderr, "FAIL chown HOME: %s\n", strerror(errno));
    else
        OK("HOME given to the mapped id");
    snprintf(t, sizeof t, "%s/old", root);
    mkdir(t, 0755);
    if (syscall(SYS_pivot_root, root, t))
        DIE("pivot_root");
    if (chdir("/") || umount2("/old", MNT_DETACH))
        DIE("detach old root");
    OK("pivot into the view");
    if (setgroups(0, 0))
        DIE("setgroups");
    if (setresgid(1000, 1000, 1000) || setresuid(1000, 1000, 1000))
        DIE("become 1000");
    OK("workload identity 1000");
    if (!setuid(0)) {
        fprintf(stderr, "FAIL regained uid 0\n");
        exit(9);
    }
    /* Cleared by the id change: /proc/self is root's until it is set back. */
    fprintf(stderr, "note dumpable after setresuid: %d\n", prctl(PR_GET_DUMPABLE));
    prctl(PR_SET_DUMPABLE, 1);
    landlock();
    OK("landlock");
    setenv("HOME", home, 1);
    setenv("USER", "devcroft", 1);
    setenv("LOGNAME", "devcroft", 1);
    if (chdir(ws))
        DIE("chdir workspace");
    execvp(argv[0], argv);
    DIE(argv[0]);
}

static int run(char *const a[]) {
    pid_t p = fork();
    if (!p) {
        execvp(a[0], a);
        _exit(127);
    }
    int s;
    waitpid(p, &s, 0);
    return WIFEXITED(s) ? WEXITSTATUS(s) : 128;
}

int main(int argc, char **argv) {
    if (argc < 6)
        return 2;
    ws = argv[3];
    home = argv[4];
    if (pipe(p2c))
        DIE("pipe");
    pid_t pid = syscall(SYS_clone,
                        CLONE_NEWUSER | CLONE_NEWNS | CLONE_NEWPID | CLONE_NEWNET |
                            CLONE_NEWIPC | SIGCHLD,
                        0, 0, 0, 0);
    if (pid < 0)
        DIE("clone");
    if (!pid)
        return child(argv + 5);
    char ps[16], u[16], g[16];
    snprintf(ps, sizeof ps, "%d", pid);
    snprintf(u, sizeof u, "%d", getuid());
    snprintf(g, sizeof g, "%d", getgid());
    char *um[] = {"newuidmap", ps, "0", u, "1", "1000", argv[1], "1", 0};
    char *gm[] = {"newgidmap", ps, "0", g, "1", "1000", argv[2], "1", 0};
    int ur = run(um), gr = run(gm);
    fprintf(stderr, "%s newuidmap %d, newgidmap %d\n", ur || gr ? "FAIL" : "ok  ", ur, gr);
    if (ur || gr)
        return 4;
    if (write(p2c[1], "x", 1) != 1)
        return 8;
    int s;
    waitpid(pid, &s, 0);
    return WIFEXITED(s) ? WEXITSTATUS(s) : 128 + WTERMSIG(s);
}
