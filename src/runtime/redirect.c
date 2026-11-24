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
#include <spawn.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

extern char **environ;

/* Saved at load, as `NAME=value`, when this process is one of devcroft's own
 * utilities (the same file as the runtime's `env`): `env -i` clears them
 * before it execs, and the program it starts must keep the redirect. Any
 * other program that clears its environment loses it, as published. */
static char dc_saved[2][PATH_MAX + 32];
static int dc_restore;

static int dc_save(char *out, size_t size, const char *name) {
    const char *v = getenv(name);
    if (!v || strlen(name) + 1 + strlen(v) + 1 > size) return 0;
    strcpy(out, name);
    strcat(out, "=");
    strcat(out, v);
    return 1;
}

__attribute__((constructor)) static void dc_init(void) {
    char exe[PATH_MAX], own[PATH_MAX];
    uint32_t size = sizeof exe;
    struct stat a, b;
    const char *ns = getenv("DEVCROFT_NS");
    static const char suffix[] = "/../runtime/env";
    if (!ns || strlen(ns) + sizeof suffix > sizeof own) return;
    strcpy(own, ns);
    strcat(own, suffix);
    if (_NSGetExecutablePath(exe, &size) != 0 || stat(exe, &a) != 0 || stat(own, &b) != 0) return;
    if (a.st_dev != b.st_dev || a.st_ino != b.st_ino) return;
    dc_restore = dc_save(dc_saved[0], sizeof dc_saved[0], "DYLD_INSERT_LIBRARIES") &&
                 dc_save(dc_saved[1], sizeof dc_saved[1], "DEVCROFT_NS");
}

static int dc_envc(char *const envp[]) {
    int n = 0;
    while (envp && envp[n]) n++;
    return n;
}

/* `envp`, or `out` (room for envc + 3) holding it with the saved variables. */
static char *const *dc_env(char *const envp[], char **out) {
    if (!dc_restore) return envp;
    int n = 0;
    for (int i = 0; envp && envp[i]; i++)
        if (strncmp(envp[i], "DYLD_INSERT_LIBRARIES=", 22) != 0 &&
            strncmp(envp[i], "DEVCROFT_NS=", 12) != 0)
            out[n++] = envp[i];
    out[n++] = dc_saved[0];
    out[n++] = dc_saved[1];
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
    const char *ns = dc_restore ? dc_saved[1] + sizeof "DEVCROFT_NS" : getenv("DEVCROFT_NS");
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
    if (!im) return path;
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

#define DC_BUFFERS                                                                          \
    char mapped[PATH_MAX], interp[DC_SHEBANG_MAX], arg[DC_SHEBANG_MAX];                     \
    int use_nv = 0;                                                                         \
    char *nv[dc_argc(argv) + 4];                                                            \
    char *ev[dc_envc(envp) + 3];

#define DC_RESOLVE(path)                                                                    \
    dc_resolve((path), argv, mapped, sizeof mapped, interp, sizeof interp, arg, sizeof arg, \
               nv, &use_nv)

static int dc_posix_spawn(pid_t *pid, const char *path, const posix_spawn_file_actions_t *fa,
                          const posix_spawnattr_t *attr, char *const argv[],
                          char *const envp[]) {
    DC_BUFFERS
    const char *target = DC_RESOLVE(path);
    return posix_spawn(pid, target, fa, attr, use_nv ? nv : argv, dc_env(envp, ev));
}

/* posix_spawnp searches PATH only for a name without `/`; an absolute or
 * relative path is the same as posix_spawn. */
static int dc_posix_spawnp(pid_t *pid, const char *file, const posix_spawn_file_actions_t *fa,
                           const posix_spawnattr_t *attr, char *const argv[],
                           char *const envp[]) {
    if (!file || !strchr(file, '/')) return posix_spawnp(pid, file, fa, attr, argv, envp);
    DC_BUFFERS
    const char *target = DC_RESOLVE(file);
    return posix_spawn(pid, target, fa, attr, use_nv ? nv : argv, dc_env(envp, ev));
}

static int dc_execve(const char *path, char *const argv[], char *const envp[]) {
    DC_BUFFERS
    const char *target = DC_RESOLVE(path);
    return execve(target, use_nv ? nv : argv, dc_env(envp, ev));
}

static int dc_execv(const char *path, char *const argv[]) {
    char *const *envp = environ;
    DC_BUFFERS
    const char *target = DC_RESOLVE(path);
    return execve(target, use_nv ? nv : argv, dc_env(envp, ev));
}

static int dc_execvp(const char *file, char *const argv[]) {
    if (!file || !strchr(file, '/')) return execvp(file, argv);
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
    return access(m ? m : path, mode);
}

static int dc_faccessat(int fd, const char *path, int mode, int flag) {
    char mapped[PATH_MAX];
    const char *m = dc_map(path, mapped, sizeof mapped);
    return faccessat(fd, m ? m : path, mode, flag);
}

static int dc_stat(const char *path, struct stat *st) {
    char mapped[PATH_MAX];
    const char *m = dc_map(path, mapped, sizeof mapped);
    return stat(m ? m : path, st);
}

/* The entry is a link of devcroft's; what it stands for is a file. */
static int dc_lstat(const char *path, struct stat *st) {
    char mapped[PATH_MAX];
    const char *m = dc_map(path, mapped, sizeof mapped);
    return m ? stat(m, st) : lstat(path, st);
}

static int dc_fstatat(int fd, const char *path, struct stat *st, int flag) {
    char mapped[PATH_MAX];
    const char *m = dc_map(path, mapped, sizeof mapped);
    return m ? stat(m, st) : fstatat(fd, path, st, flag);
}

/* dyld applies these to every image but this one, so the calls above reach
 * the real functions. Calls made inside libSystem (system(), popen(), the
 * execl family) are interposed too (measured, design.md D1). */
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
};
