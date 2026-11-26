/* Measurement only. Seatbelt enforces the policy in run.py. */
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <spawn.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <dirent.h>
#include <unistd.h>

static const char *root;
static const char *work;
static size_t root_len;
static int trace;

__attribute__((constructor)) static void init(void) {
    root = getenv("DC_TEMP");
    root_len = root ? strlen(root) : 0;
    work = getenv("DC_WORK");
    trace = getenv("DC_TRACE") != NULL;
    if (trace) fputs("temp interposer loaded\n", stderr);
}

static const char *suffix(const char *path, const char *prefix) {
    size_t n = strlen(prefix);
    if (strncmp(path, prefix, n) != 0 || (path[n] != '/' && path[n] != '\0'))
        return NULL;
    return path + n;
}

static const char *map(const char *call, const char *path, char out[PATH_MAX]) {
    if (!root || !path || path[0] != '/') return path;
    const char *rest = suffix(path, "/tmp");
    if (!rest) rest = suffix(path, "/private/tmp");
    if (!rest) return path;
    if (work && suffix(rest, work)) return path;
    size_t rest_len = strlen(rest);
    if (root_len + rest_len >= PATH_MAX) return path;
    memcpy(out, root, root_len);
    memcpy(out + root_len, rest, rest_len + 1);
    if (trace) fprintf(stderr, "%s: %s -> %s\n", call, path, out);
    return out;
}

#define ONE(type, name, args, ...)                                 \
    static type wrap_##name args {                                  \
        char out[PATH_MAX];                                         \
        return name(__VA_ARGS__);                                  \
    }
#define PATH map(__func__ + 5, path, out)

ONE(int, access, (const char *path, int mode), PATH, mode)
ONE(int, faccessat, (int fd, const char *path, int mode, int flag), fd, PATH, mode, flag)
ONE(int, stat, (const char *path, struct stat *st), PATH, st)
ONE(int, lstat, (const char *path, struct stat *st), PATH, st)
ONE(int, fstatat, (int fd, const char *path, struct stat *st, int flag), fd, PATH, st, flag)
ONE(int, mkdir, (const char *path, mode_t mode), PATH, mode)
ONE(int, mkdirat, (int fd, const char *path, mode_t mode), fd, PATH, mode)
ONE(int, rmdir, (const char *path), PATH)
ONE(int, chdir, (const char *path), PATH)
ONE(DIR *, opendir, (const char *path), PATH)
ONE(int, unlink, (const char *path), PATH)
ONE(int, unlinkat, (int fd, const char *path, int flags), fd, PATH, flags)
ONE(int, chmod, (const char *path, mode_t mode), PATH, mode)
ONE(int, fchmodat, (int fd, const char *path, mode_t mode, int flag), fd, PATH, mode, flag)
ONE(char *, realpath, (const char *path, char *dest), PATH, dest)
ONE(ssize_t, readlink, (const char *path, char *dest, size_t size), PATH, dest, size)
ONE(ssize_t, readlinkat, (int fd, const char *path, char *dest, size_t size), fd, PATH, dest, size)
ONE(FILE *, fopen, (const char *path, const char *mode), PATH, mode)
ONE(int, execve, (const char *path, char *const argv[], char *const envp[]), PATH, argv, envp)
ONE(int, posix_spawn, (pid_t *pid, const char *path, const posix_spawn_file_actions_t *fa,
                      const posix_spawnattr_t *attr, char *const argv[], char *const envp[]),
    pid, PATH, fa, attr, argv, envp)

#define MODE                                    \
    int mode = 0;                               \
    if (flags & O_CREAT) {                      \
        va_list ap;                             \
        va_start(ap, flags);                    \
        mode = va_arg(ap, int);                 \
        va_end(ap);                             \
    }
static int wrap_open(const char *path, int flags, ...) {
    MODE
    char out[PATH_MAX];
    return open(map("open", path, out), flags, mode);
}

static int wrap_openat(int fd, const char *path, int flags, ...) {
    MODE
    char out[PATH_MAX];
    return openat(fd, map("openat", path, out), flags, mode);
}

#define TWO(name, args, ...)                                                \
    static int wrap_##name args {                                            \
        char a[PATH_MAX], b[PATH_MAX];                                       \
        const char *from = map(#name, src, a);                               \
        const char *to = map(#name, dest, b);                                \
        return name(__VA_ARGS__);                                           \
    }

TWO(rename, (const char *src, const char *dest), from, to)
TWO(renameat, (int fd, const char *src, int destfd, const char *dest), fd, from, destfd, to)
TWO(symlink, (const char *src, const char *dest), from, to)
TWO(symlinkat, (const char *src, int fd, const char *dest), from, fd, to)

static void copy_template(char *logical, const char *physical) {
    /* mk*temp changes only the Xs at the end of the template. */
    memcpy(logical + strlen(logical) - 6, physical + strlen(physical) - 6, 6);
}

static int wrap_mkstemp(char *path) {
    char out[PATH_MAX];
    if (map("mkstemp", path, out) == path) return mkstemp(path);
    int result = mkstemp(out);
    if (result >= 0) copy_template(path, out);
    return result;
}

static char *wrap_mkdtemp(char *path) {
    char out[PATH_MAX];
    if (map("mkdtemp", path, out) == path) return mkdtemp(path);
    if (!mkdtemp(out)) return NULL;
    copy_template(path, out);
    return path;
}

static const struct sockaddr *map_socket(const struct sockaddr *address, socklen_t *len,
                                         struct sockaddr_un *out, const char *call) {
    if (!address || *len < sizeof address->sa_len + sizeof address->sa_family ||
        address->sa_family != AF_UNIX)
        return address;
    struct sockaddr_un original = {0};
    memcpy(&original, address, *len < sizeof original ? *len : sizeof original);
    char path[sizeof original.sun_path + 1] = {0}, mapped[PATH_MAX];
    memcpy(path, original.sun_path, sizeof original.sun_path);
    if (map(call, path, mapped) == path) return address;
    if (strlen(mapped) >= sizeof out->sun_path) {
        errno = ENAMETOOLONG;
        return NULL;
    }
    out->sun_family = AF_UNIX;
    strcpy(out->sun_path, mapped);
    *len = SUN_LEN(out);
    out->sun_len = *len;
    return (const struct sockaddr *)out;
}

#define SOCKET(name)                                                              \
    static int wrap_##name(int fd, const struct sockaddr *address, socklen_t len) { \
        struct sockaddr_un out = {0};                                             \
        const struct sockaddr *mapped = map_socket(address, &len, &out, #name);    \
        return mapped ? name(fd, mapped, len) : -1;                               \
    }
SOCKET(bind)
SOCKET(connect)

#define PAIR(name) {(const void *)wrap_##name, (const void *)name}
__attribute__((used, section("__DATA,__interpose")))
static const struct { const void *replacement; const void *original; } hooks[] = {
    PAIR(access), PAIR(faccessat), PAIR(stat), PAIR(lstat), PAIR(fstatat),
    PAIR(mkdir), PAIR(mkdirat), PAIR(rmdir), PAIR(chdir), PAIR(opendir), PAIR(unlink), PAIR(unlinkat),
    PAIR(chmod), PAIR(fchmodat), PAIR(realpath), PAIR(readlink), PAIR(readlinkat),
    PAIR(fopen), PAIR(open), PAIR(openat), PAIR(rename), PAIR(renameat),
    PAIR(symlink), PAIR(symlinkat), PAIR(mkstemp), PAIR(mkdtemp),
    PAIR(bind), PAIR(connect), PAIR(execve), PAIR(posix_spawn),
};
