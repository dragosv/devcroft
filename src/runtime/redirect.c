/*
 * devcroft's macOS exec redirect (`add-macos-exec-redirect`).
 *
 * Loaded into sandbox processes through DYLD_INSERT_LIBRARIES. An exec of a
 * base-name path (`/bin/<name>`, `/usr/bin/<name>`, `/bin/sh`,
 * `/usr/bin/env`) that has an entry in the sandbox's namespace directory
 * (DEVCROFT_NS) runs that entry instead: the environment's executable where it
 * supplies the name, devcroft's runtime otherwise, as `up` computed it (rule B).
 * A script whose `#!` interpreter is such a path is started with the entry,
 * with the kernel's argument convention. Everything else reaches the real call
 * unchanged, and Seatbelt decides.
 *
 * Compatibility, not a boundary: a process that drops the variable, or a
 * binary dyld will not load it into, gets Seatbelt's EPERM, never the host.
 *
 * No logging, no allocation: the exec family can be called between vfork and
 * exec, so everything lives on the stack.
 */
#include <errno.h>
#include <fcntl.h>
#include <mach-o/dyld.h>
#include <limits.h>
#include <pwd.h>
#include <spawn.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <utime.h>

#include "redirect_temp.h"

extern char **environ;

/* Saved at load, as `NAME=value`, when this process is one of devcroft's own
 * utilities (the same file as the runtime's `env`): `env -i` clears them
 * before it execs, and the program it starts must keep the redirect. Any
 * other program that clears its environment loses it, as published. */
static const char *const dc_kept[] = {
    "DYLD_INSERT_LIBRARIES", "DEVCROFT_NS", "DEVCROFT_TEMP", "DEVCROFT_TEMP_GRANTS",
    "DEVCROFT_ID_USER", "DEVCROFT_ID_HOME", "DEVCROFT_ID_SHELL", "DEVCROFT_EXEC_LOG",
};
#define DC_KEPT (sizeof dc_kept / sizeof dc_kept[0])
static char *dc_saved[DC_KEPT];
static const char *dc_ns;
static struct passwd dc_user;
static const char *dc_exec_log;

static void dc_init_user(void) {
    const char *name = getenv("DEVCROFT_ID_USER");
    const char *home = getenv("DEVCROFT_ID_HOME");
    const char *shell = getenv("DEVCROFT_ID_SHELL");
    if (!name || !home || !shell) return;
    dc_user.pw_name = strdup(name);
    dc_user.pw_dir = strdup(home);
    dc_user.pw_shell = strdup(shell);
    dc_user.pw_uid = getuid();
    dc_user.pw_gid = getgid();
    dc_user.pw_passwd = "*";
    dc_user.pw_class = "";
    dc_user.pw_gecos = dc_user.pw_name;
    if (!dc_user.pw_dir || !dc_user.pw_shell) dc_user.pw_name = NULL;
}

__attribute__((constructor)) static void dc_init(void) {
    dc_init_temp();
    dc_init_user();
    const char *log = getenv("DEVCROFT_EXEC_LOG");
    if (log && log[0] == '/') dc_exec_log = strdup(log);
    char exe[PATH_MAX], own[PATH_MAX];
    uint32_t size = sizeof exe;
    struct stat a, b;
    const char *ns = getenv("DEVCROFT_NS");
    if (ns) dc_ns = strdup(ns);
    static const char suffix[] = "/../runtime/env";
    if (!ns || strlen(ns) + sizeof suffix > sizeof own) return;
    strcpy(own, ns);
    strcat(own, suffix);
    if (_NSGetExecutablePath(exe, &size) != 0 || stat(exe, &a) != 0 || stat(own, &b) != 0) return;
    if (a.st_dev != b.st_dev || a.st_ino != b.st_ino) return;
    for (size_t i = 0; i < DC_KEPT; ++i) {
        const char *name = dc_kept[i];
        const char *value = getenv(name);
        if (value && asprintf(&dc_saved[i], "%s=%s", name, value) < 0)
            dc_saved[i] = NULL;
    }
}

static int dc_envc(char *const envp[]) {
    int n = 0;
    while (envp && envp[n]) n++;
    return n;
}

static int dc_replaced(const char *entry) {
    for (size_t i = 0; i < DC_KEPT; ++i) {
        size_t len = strlen(dc_kept[i]);
        if (dc_saved[i] && strncmp(entry, dc_kept[i], len) == 0 && entry[len] == '=')
            return 1;
    }
    return 0;
}

/* `envp`, or `out` (room for envc + DC_KEPT + 1) with the saved variables. */
static char *const *dc_env(char *const envp[], char **out) {
    if (!dc_saved[0]) return envp;
    int n = 0;
    for (int i = 0; envp && envp[i]; i++)
        if (!dc_replaced(envp[i]))
            out[n++] = envp[i];
    for (size_t i = 0; i < DC_KEPT; ++i)
        if (dc_saved[i]) out[n++] = dc_saved[i];
    out[n] = NULL;
    return out;
}

/* The longest `#!` line read, as the kernel's own limit (BINPRM_BUF_SIZE on
 * Linux; XNU reads 512 bytes). */
#define DC_SHEBANG_MAX 512

/* The base name an absolute path stands for, or NULL. A name never contains
 * `/`, so `/usr/bin/x/y` is not one. */
static const char *dc_base_name(const char *path) {
    const char *rest = NULL;
    if (!path) return NULL;
    if (strncmp(path, "/bin/", 5) == 0) rest = path + 5;
    else if (strncmp(path, "/usr/bin/", 9) == 0) rest = path + 9;
    if (!rest || !*rest || strchr(rest, '/')) return NULL;
    if (strcmp(rest, ".") == 0 || strcmp(rest, "..") == 0) return NULL;
    return rest;
}

/* The namespace entry for `path` into `out`, or NULL when there is none (no
 * namespace, not a base-name path, no entry, or an entry that is not
 * executable). */
static const char *dc_map(const char *path, char *out, size_t size) {
    const char *ns = dc_saved[1] ? dc_ns : getenv("DEVCROFT_NS");
    const char *name = dc_base_name(path);
    if (!ns || !*ns || !name) return NULL;
    size_t need = strlen(ns) + 1 + strlen(name) + 1;
    if (need > size) return NULL;
    strcpy(out, ns);
    strcat(out, "/");
    strcat(out, name);
    if (access(out, X_OK) != 0) return NULL;
    return out;
}

/*
 * Parse the `#!` line of `path`: the interpreter into `interp`, its one
 * optional argument into `arg` (empty when absent). The kernel's convention:
 * whitespace after `#!` skipped, the interpreter ends at the first space or
 * tab, everything after it (leading whitespace skipped, trailing whitespace
 * and a CR dropped) is one argument. Returns 0 on success, -1 when the file
 * is not a script this can parse (it then runs unchanged).
 */
static int dc_shebang(const char *path, char *interp, size_t isize, char *arg, size_t asize) {
    char head[DC_SHEBANG_MAX + 1];
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return -1;
    ssize_t n = read(fd, head, DC_SHEBANG_MAX);
    int saved = errno;
    close(fd);
    errno = saved;
    if (n < 3 || head[0] != '#' || head[1] != '!') return -1;
    head[n] = '\0';
    char *end = memchr(head, '\n', (size_t)n);
    if (end) *end = '\0';
    else if (n == DC_SHEBANG_MAX) return -1; /* a line longer than the limit */
    size_t len = strlen(head);
    while (len > 2 && (head[len - 1] == '\r' || head[len - 1] == ' ' || head[len - 1] == '\t'))
        head[--len] = '\0';
    char *s = head + 2;
    while (*s == ' ' || *s == '\t') s++;
    if (!*s) return -1;
    char *a = s;
    while (*a && *a != ' ' && *a != '\t') a++;
    if (*a) {
        *a++ = '\0';
        while (*a == ' ' || *a == '\t') a++;
    }
    if (strlen(s) + 1 > isize || strlen(a) + 1 > asize) return -1;
    strcpy(interp, s);
    strcpy(arg, a);
    return 0;
}

static int dc_argc(char *const argv[]) {
    int n = 0;
    while (argv && argv[n]) n++;
    return n;
}

/*
 * What to run for `path` with `argv`: the mapped entry, the emulated shebang,
 * or `path` itself. Fills `nv` (room for argc + 4 entries) when it rewrites
 * the arguments. Returns the path to execute and sets *use_nv.
 */
static const char *dc_resolve(const char *path, char *const argv[], char *mapped, size_t msize,
                              char *interp, size_t isize, char *arg, size_t asize, char **nv,
                              int *use_nv) {
    *use_nv = 0;
    const char *m = dc_map(path, mapped, msize);
    if (m) return m;
    /* Only a script whose interpreter maps is rewritten; any other file is
     * left to the kernel, including its own shebang handling. */
    if (!path || dc_shebang(path, interp, isize, arg, asize) != 0) return path;
    const char *im = dc_map(interp, mapped, msize);
    if (!im) {
        im = dc_temp_path(interp, mapped);
        if (im == interp) return path;
    }
    int argc = dc_argc(argv);
    int i = 0;
    nv[i++] = interp;
    if (*arg) nv[i++] = arg;
    nv[i++] = (char *)path;
    for (int k = 1; k < argc; k++) nv[i++] = argv[k];
    nv[i] = NULL;
    *use_nv = 1;
    return im;
}

/*
 * One line per exec, when `up` was asked for it (`DEVCROFT_LOG_EXECS`): the
 * path as the caller named it, then each argument, tab-separated, with `\`,
 * tab and newline escaped. Written before the exec, so a refused or missing
 * program is recorded too. One write(2) on an O_APPEND file, from a stack
 * buffer: no allocation, since a child of vfork may call this. A line longer
 * than the buffer ends in `\t[truncated]`. Failure to log never fails the exec.
 */
#define DC_LOG_MAX 16384
/* Set while libSystem searches `PATH` for a bare name: it tries an execve or
 * posix_spawn per directory, each of which would log again (measured: one
 * line per directory tried, including the misses). */
static __thread int dc_searching;
static void dc_log_exec(const char *path, char *const argv[]) {
    if (!dc_exec_log || !path || dc_searching) return;
    /* A miss in a program's own PATH search (Python's subprocess, libuv and
     * Bun walk it themselves): a bare argv[0], a path ending in that name,
     * and nothing there to run. A miss for a path the caller named in full
     * is still logged. */
    const char *slash = strrchr(path, '/');
    struct stat st;
    if (slash && argv && argv[0] && !strchr(argv[0], '/') && strcmp(slash + 1, argv[0]) == 0 &&
        stat(path, &st) != 0)
        return;
    static const char cut[] = "\t[truncated]\n";
    char line[DC_LOG_MAX];
    const size_t room = sizeof line - sizeof cut;
    size_t n = 0;
    int truncated = 0;
    for (int i = -1; !truncated && (i < 0 || (argv && argv[i])); i++) {
        const char *s = i < 0 ? path : argv[i];
        if (i >= 0) line[n++] = '\t';
        for (; *s; s++) {
            const char *esc = *s == '\\' ? "\\\\" : *s == '\t' ? "\\t" : *s == '\n' ? "\\n" : NULL;
            size_t len = esc ? 2 : 1;
            if (n + len + 1 > room) {
                truncated = 1;
                break;
            }
            if (esc) {
                line[n++] = esc[0];
                line[n++] = esc[1];
            } else {
                line[n++] = *s;
            }
        }
    }
    if (truncated) {
        memcpy(line + n, cut, sizeof cut - 1);
        n += sizeof cut - 1;
    } else {
        line[n++] = '\n';
    }
    int fd = open(dc_exec_log, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0600);
    if (fd < 0) return;
    (void)!write(fd, line, n);
    close(fd);
}

#define DC_BUFFERS                                                                          \
    char mapped[PATH_MAX], temp[PATH_MAX], interp[DC_SHEBANG_MAX], arg[DC_SHEBANG_MAX];     \
    int use_nv = 0;                                                                         \
    char *nv[dc_argc(argv) + 4];                                                            \
    char *ev[dc_envc(envp) + DC_KEPT + 1];

#define DC_RESOLVE(path)                                                                    \
    dc_resolve(dc_temp_path((path), temp), argv, mapped, sizeof mapped, interp, sizeof interp, arg, sizeof arg, \
               nv, &use_nv)

static int dc_posix_spawn(pid_t *pid, const char *path, const posix_spawn_file_actions_t *fa,
                          const posix_spawnattr_t *attr, char *const argv[],
                          char *const envp[]) {
    dc_log_exec(path, argv);
    DC_BUFFERS
    const char *target = DC_RESOLVE(path);
    return posix_spawn(pid, target, fa, attr, use_nv ? nv : argv, dc_env(envp, ev));
}

/* posix_spawnp searches PATH only for a name without `/`; an absolute or
 * relative path is the same as posix_spawn. */
static int dc_posix_spawnp(pid_t *pid, const char *file, const posix_spawn_file_actions_t *fa,
                           const posix_spawnattr_t *attr, char *const argv[],
                           char *const envp[]) {
    dc_log_exec(file, argv);
    if (!file || !strchr(file, '/')) {
        char *ev[dc_envc(envp) + DC_KEPT + 1];
        dc_searching = 1;
        int rc = posix_spawnp(pid, file, fa, attr, argv, dc_env(envp, ev));
        dc_searching = 0;
        return rc;
    }
    DC_BUFFERS
    const char *target = DC_RESOLVE(file);
    return posix_spawn(pid, target, fa, attr, use_nv ? nv : argv, dc_env(envp, ev));
}

static int dc_execve(const char *path, char *const argv[], char *const envp[]) {
    dc_log_exec(path, argv);
    DC_BUFFERS
    const char *target = DC_RESOLVE(path);
    return execve(target, use_nv ? nv : argv, dc_env(envp, ev));
}

static int dc_execv(const char *path, char *const argv[]) {
    dc_log_exec(path, argv);
    char *const *envp = environ;
    DC_BUFFERS
    const char *target = DC_RESOLVE(path);
    return execve(target, use_nv ? nv : argv, dc_env(envp, ev));
}

static int dc_execvp(const char *file, char *const argv[]) {
    dc_log_exec(file, argv);
    if (!file || !strchr(file, '/')) {
        dc_searching = 1;
        int rc = execvp(file, argv);
        dc_searching = 0;
        return rc;
    }
    char *const *envp = environ;
    DC_BUFFERS
    const char *target = DC_RESOLVE(file);
    return execve(target, use_nv ? nv : argv, dc_env(envp, ev));
}

/*
 * Asking about a path gets the answer executing it would: a program that
 * checks `access("/bin/ls", X_OK)` before it execs (GNU make, through gnulib)
 * is refused by the sandbox there, and never reaches the exec. Nothing is
 * opened for it: reading `/bin/ls` stays refused.
 */
static int dc_access(const char *path, int mode) {
    char mapped[PATH_MAX];
    const char *m = dc_map(path, mapped, sizeof mapped);
    return access(m ? m : dc_temp_path(path, mapped), mode);
}

static int dc_faccessat(int fd, const char *path, int mode, int flag) {
    char mapped[PATH_MAX];
    const char *m = dc_map(path, mapped, sizeof mapped);
    return faccessat(fd, m ? m : dc_temp_path(path, mapped), mode, flag);
}

static int dc_stat(const char *path, struct stat *st) {
    char mapped[PATH_MAX];
    const char *m = dc_map(path, mapped, sizeof mapped);
    return stat(m ? m : dc_temp_path(path, mapped), st);
}

/* The entry is a link of devcroft's; what it stands for is a file. */
static int dc_lstat(const char *path, struct stat *st) {
    char mapped[PATH_MAX];
    const char *m = dc_map(path, mapped, sizeof mapped);
    return m ? stat(m, st) : lstat(dc_temp_path(path, mapped), st);
}

static int dc_fstatat(int fd, const char *path, struct stat *st, int flag) {
    char mapped[PATH_MAX];
    const char *m = dc_map(path, mapped, sizeof mapped);
    return m ? stat(m, st) : fstatat(fd, dc_temp_path(path, mapped), st, flag);
}

static struct passwd *dc_getpwuid(uid_t uid) {
    return dc_user.pw_name && dc_user.pw_uid == uid ? &dc_user : getpwuid(uid);
}

static struct passwd *dc_getpwnam(const char *name) {
    return dc_user.pw_name && strcmp(dc_user.pw_name, name) == 0 ? &dc_user : getpwnam(name);
}

static int dc_copy_user(struct passwd *user, char *buf, size_t size, struct passwd **result) {
    *result = NULL;
    *user = dc_user;
    char **fields[] = {&user->pw_name, &user->pw_passwd, &user->pw_class,
                       &user->pw_gecos, &user->pw_dir, &user->pw_shell};
    for (size_t i = 0; i < sizeof fields / sizeof fields[0]; ++i) {
        size_t len = strlen(*fields[i]) + 1;
        if (len > size) return ERANGE;
        memcpy(buf, *fields[i], len);
        *fields[i] = buf;
        buf += len;
        size -= len;
    }
    *result = user;
    return 0;
}

static int dc_getpwuid_r(uid_t uid, struct passwd *user, char *buf, size_t size,
                         struct passwd **result) {
    if (dc_user.pw_name && dc_user.pw_uid == uid) return dc_copy_user(user, buf, size, result);
    return getpwuid_r(uid, user, buf, size, result);
}

static int dc_getpwnam_r(const char *name, struct passwd *user, char *buf, size_t size,
                         struct passwd **result) {
    if (dc_user.pw_name && strcmp(dc_user.pw_name, name) == 0)
        return dc_copy_user(user, buf, size, result);
    return getpwnam_r(name, user, buf, size, result);
}

/* dyld applies these to every image but this one, so the calls above reach
 * the real functions. Calls made inside libSystem (system(), popen(), the
 * execl family) are interposed too (measured, design.md D1). */
#define DC_PAIR(name) {(const void *)dc_##name, (const void *)name}
__attribute__((used)) static const struct {
    const void *replacement;
    const void *original;
} dc_interposers[] __attribute__((section("__DATA,__interpose"))) = {
    {(const void *)dc_posix_spawn, (const void *)posix_spawn},
    {(const void *)dc_posix_spawnp, (const void *)posix_spawnp},
    {(const void *)dc_execve, (const void *)execve},
    {(const void *)dc_execv, (const void *)execv},
    {(const void *)dc_execvp, (const void *)execvp},
    {(const void *)dc_access, (const void *)access},
    {(const void *)dc_faccessat, (const void *)faccessat},
    {(const void *)dc_stat, (const void *)stat},
    {(const void *)dc_lstat, (const void *)lstat},
    {(const void *)dc_fstatat, (const void *)fstatat},
    DC_PAIR(open), DC_PAIR(openat), DC_PAIR(creat),
    DC_PAIR(chmod), DC_PAIR(lchmod), DC_PAIR(fchmodat), DC_PAIR(chown), DC_PAIR(lchown),
    DC_PAIR(fchownat), DC_PAIR(utime), DC_PAIR(utimes), DC_PAIR(lutimes), DC_PAIR(utimensat),
    DC_PAIR(getxattr), DC_PAIR(setxattr), DC_PAIR(listxattr), DC_PAIR(removexattr),
    DC_PAIR(mkdir), DC_PAIR(mkdirat), DC_PAIR(rmdir), DC_PAIR(chdir), DC_PAIR(opendir),
    DC_PAIR(unlink), DC_PAIR(unlinkat), DC_PAIR(truncate), DC_PAIR(realpath),
    DC_PAIR(readlink), DC_PAIR(readlinkat), DC_PAIR(fopen), DC_PAIR(freopen),
    DC_PAIR(rename), DC_PAIR(renameat), DC_PAIR(renameatx_np),
    DC_PAIR(link), DC_PAIR(linkat), DC_PAIR(symlink), DC_PAIR(symlinkat),
    DC_PAIR(mkstemp), DC_PAIR(mkstemps), DC_PAIR(mkostemp), DC_PAIR(mkostemps), DC_PAIR(mkdtemp),
    DC_PAIR(bind), DC_PAIR(connect), DC_PAIR(posix_spawn_file_actions_addopen),
    DC_PAIR(posix_spawn_file_actions_addchdir_np),
    DC_PAIR(getpwuid), DC_PAIR(getpwnam), DC_PAIR(getpwuid_r), DC_PAIR(getpwnam_r),
};
