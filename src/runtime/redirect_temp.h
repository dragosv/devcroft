/* macOS path compatibility for redirect.c. Seatbelt is the boundary. */
#include <dirent.h>
#include <stddef.h>
#include <stdio.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <sys/un.h>
#include <sys/xattr.h>

static char dc_temp[PATH_MAX];
static size_t dc_temp_len;
static const char *dc_temp_grants;
static char dc_long_path[PATH_MAX + 1];
static const char dc_host_tmp[] = "/private/tmp";

static const char *dc_suffix(const char *path, const char *prefix, size_t len) {
    if (strncmp(path, prefix, len) || (path[len] != '/' && path[len] != '\0')) return NULL;
    return path + len;
}

/* Canonicalize the spelling of a literal temp path without touching disk.
 * Ordinary filenames with neither spelling nor parent steps take the fast path.
 * A trailing slash is kept by dc_temp_path so open still requires a directory. */
static const char *dc_temp_suffix(const char *path, char normalized[PATH_MAX]) {
    if (!dc_suffix(path, "/tmp", 4) &&
        !dc_suffix(path, dc_host_tmp, sizeof dc_host_tmp - 1) &&
        path[1] != '.' && path[1] != '/' && !strstr(path, "/.."))
        return NULL;
    const char *p = path;
    size_t end = 0;
    while (*p) {
        if (*p == '/') { p++; continue; }
        size_t len = strcspn(p, "/");
        if (len == 2 && p[0] == '.' && p[1] == '.') {
            while (end && normalized[--end] != '/') {}
        } else if (len != 1 || p[0] != '.') {
            if (end + 1 + len >= PATH_MAX) return NULL;
            normalized[end++] = '/';
            memcpy(normalized + end, p, len);
            end += len;
        }
        p += len;
    }
    normalized[end] = '\0';
    const char *suffix = dc_suffix(normalized, "/tmp", 4);
    return suffix ? suffix : dc_suffix(normalized, dc_host_tmp, sizeof dc_host_tmp - 1);
}

static const char *dc_temp_path(const char *path, char out[PATH_MAX]) {
    if (!dc_temp_len || !path || path[0] != '/') return path;
    char normalized[PATH_MAX];
    const char *suffix = dc_temp_suffix(path, normalized);
    if (!suffix) return path;
    size_t len = strlen(suffix), host_len = sizeof dc_host_tmp - 1;
    if (host_len + len >= PATH_MAX) return dc_long_path;
    memcpy(out, dc_host_tmp, host_len);
    memcpy(out + host_len, suffix, len + 1);
    const char *base = dc_temp;
    size_t base_len = dc_temp_len;
    const char *grants = dc_temp_grants;
    while (grants && *grants) {
        size_t n = 0;
        while (*grants >= '0' && *grants <= '9' && n < PATH_MAX)
            n = n * 10 + (size_t)(*grants++ - '0');
        if (*grants++ != ':' || !n || n >= PATH_MAX || strlen(grants) < n) break;
        if (dc_suffix(out, grants, n)) {
            if (n == host_len) return path; /* the whole host temp was granted */
            base = dc_host_tmp;
            base_len = host_len;
            break;
        }
        grants += n;
    }
    size_t path_len = strlen(path);
    int slash = path[path_len - 1] == '/' ||
                (path_len >= 2 && strcmp(path + path_len - 2, "/.") == 0) ||
                (path_len >= 3 && strcmp(path + path_len - 3, "/..") == 0);
    if (base_len + len + (size_t)slash >= PATH_MAX) return dc_long_path;
    memcpy(out, base, base_len);
    memcpy(out + base_len, suffix, len);
    if (slash) out[base_len + len++] = '/';
    out[base_len + len] = '\0';
    return out;
}

static void dc_init_temp(void) {
    const char *temp = getenv("DEVCROFT_TEMP");
    if (!temp || !*temp || strlen(temp) >= sizeof dc_temp) return;
    strcpy(dc_temp, temp);
    dc_temp_len = strlen(temp);
    const char *grants = getenv("DEVCROFT_TEMP_GRANTS");
    if (grants) dc_temp_grants = strdup(grants);
    memset(dc_long_path, '/', sizeof dc_long_path - 1);
}

#define DC_WRAP_PATH(type, name, args, ...)          \
    static type dc_##name args {                  \
        char mapped[PATH_MAX];                    \
        return name(__VA_ARGS__);                 \
    }
#define DC_PATH dc_temp_path(path, mapped)

DC_WRAP_PATH(int, creat, (const char *path, mode_t mode), DC_PATH, mode)
DC_WRAP_PATH(int, chmod, (const char *path, mode_t mode), DC_PATH, mode)
DC_WRAP_PATH(int, lchmod, (const char *path, mode_t mode), DC_PATH, mode)
DC_WRAP_PATH(int, fchmodat, (int fd, const char *path, mode_t mode, int flags),
             fd, DC_PATH, mode, flags)
DC_WRAP_PATH(int, chown, (const char *path, uid_t uid, gid_t gid), DC_PATH, uid, gid)
DC_WRAP_PATH(int, lchown, (const char *path, uid_t uid, gid_t gid), DC_PATH, uid, gid)
DC_WRAP_PATH(int, fchownat, (int fd, const char *path, uid_t uid, gid_t gid, int flags),
             fd, DC_PATH, uid, gid, flags)
DC_WRAP_PATH(int, utime, (const char *path, const struct utimbuf *times), DC_PATH, times)
DC_WRAP_PATH(int, utimes, (const char *path, const struct timeval times[2]), DC_PATH, times)
DC_WRAP_PATH(int, lutimes, (const char *path, const struct timeval times[2]), DC_PATH, times)
DC_WRAP_PATH(int, utimensat, (int fd, const char *path, const struct timespec times[2], int flags),
             fd, DC_PATH, times, flags)
DC_WRAP_PATH(ssize_t, getxattr, (const char *path, const char *name, void *val, size_t size,
                                uint32_t pos, int flags), DC_PATH, name, val, size, pos, flags)
DC_WRAP_PATH(int, setxattr, (const char *path, const char *name, const void *val, size_t size,
                             uint32_t pos, int flags), DC_PATH, name, val, size, pos, flags)
DC_WRAP_PATH(ssize_t, listxattr, (const char *path, char *name, size_t size, int flags),
             DC_PATH, name, size, flags)
DC_WRAP_PATH(int, removexattr, (const char *path, const char *name, int flags), DC_PATH, name, flags)
DC_WRAP_PATH(int, mkdir, (const char *path, mode_t mode), DC_PATH, mode)
DC_WRAP_PATH(int, mkdirat, (int fd, const char *path, mode_t mode), fd, DC_PATH, mode)
DC_WRAP_PATH(int, rmdir, (const char *path), DC_PATH)
DC_WRAP_PATH(int, chdir, (const char *path), DC_PATH)
DC_WRAP_PATH(DIR *, opendir, (const char *path), DC_PATH)
DC_WRAP_PATH(int, unlink, (const char *path), DC_PATH)
DC_WRAP_PATH(int, unlinkat, (int fd, const char *path, int flags), fd, DC_PATH, flags)
DC_WRAP_PATH(int, truncate, (const char *path, off_t size), DC_PATH, size)
DC_WRAP_PATH(char *, realpath, (const char *path, char *dest), DC_PATH, dest)
DC_WRAP_PATH(ssize_t, readlink, (const char *path, char *dest, size_t size), DC_PATH, dest, size)
DC_WRAP_PATH(ssize_t, readlinkat, (int fd, const char *path, char *dest, size_t size),
             fd, DC_PATH, dest, size)
DC_WRAP_PATH(FILE *, fopen, (const char *path, const char *mode), DC_PATH, mode)
DC_WRAP_PATH(FILE *, freopen, (const char *path, const char *mode, FILE *file), DC_PATH, mode, file)
DC_WRAP_PATH(int, posix_spawn_file_actions_addopen,
             (posix_spawn_file_actions_t *fa, int fd, const char *path, int flags, mode_t mode),
             fa, fd, DC_PATH, flags, mode)

/* addchdir_np is optional at runtime on older macOS versions. */
DC_WRAP_PATH(int, posix_spawn_file_actions_addchdir_np,
             (posix_spawn_file_actions_t *fa, const char *path), fa, DC_PATH)

#define DC_OPEN_MODE                           \
    int mode = 0;                            \
    if (flags & O_CREAT) {                   \
        va_list ap;                          \
        va_start(ap, flags);                 \
        mode = va_arg(ap, int);              \
        va_end(ap);                          \
    }

static int dc_open(const char *path, int flags, ...) {
    DC_OPEN_MODE
    char mapped[PATH_MAX];
    return open(DC_PATH, flags, mode);
}

static int dc_openat(int fd, const char *path, int flags, ...) {
    DC_OPEN_MODE
    char mapped[PATH_MAX];
    return openat(fd, DC_PATH, flags, mode);
}

#define DC_WRAP_TWO(name, args, ...)                      \
    static int dc_##name args {                           \
        char a[PATH_MAX], b[PATH_MAX];                    \
        const char *from = dc_temp_path(src, a);           \
        const char *to = dc_temp_path(dest, b);            \
        return name(__VA_ARGS__);                        \
    }

DC_WRAP_TWO(rename, (const char *src, const char *dest), from, to)
DC_WRAP_TWO(renameat, (int fd, const char *src, int tofd, const char *dest), fd, from, tofd, to)
DC_WRAP_TWO(renameatx_np, (int fd, const char *src, int tofd, const char *dest, unsigned flags),
            fd, from, tofd, to, flags)
DC_WRAP_TWO(link, (const char *src, const char *dest), from, to)
DC_WRAP_TWO(linkat, (int fd, const char *src, int tofd, const char *dest, int flags),
            fd, from, tofd, to, flags)
DC_WRAP_TWO(symlink, (const char *src, const char *dest), from, to)
DC_WRAP_TWO(symlinkat, (const char *src, int fd, const char *dest), from, fd, to)

static void dc_copy_template(char *path, const char *mapped, int suffix) {
    size_t tail = 6 + (size_t)suffix;
    size_t original_len = strlen(path), mapped_len = strlen(mapped);
    if (suffix >= 0 && tail <= original_len && tail <= mapped_len)
        memcpy(path + original_len - tail, mapped + mapped_len - tail, 6);
}

#define DC_WRAP_MKSTEMP(name, args, suffix, ...)                           \
    static int dc_##name args {                                         \
        char mapped[PATH_MAX];                                         \
        const char *target = DC_PATH;                                   \
        if (target == path) return name(path, ##__VA_ARGS__);             \
        if (target == dc_long_path) { errno = ENAMETOOLONG; return -1; } \
        int result = name(mapped, ##__VA_ARGS__);                       \
        if (result >= 0) dc_copy_template(path, mapped, suffix);        \
        return result;                                                  \
    }
DC_WRAP_MKSTEMP(mkstemp, (char *path), 0)
DC_WRAP_MKSTEMP(mkstemps, (char *path, int suffix), suffix, suffix)
DC_WRAP_MKSTEMP(mkostemp, (char *path, int flags), 0, flags)
DC_WRAP_MKSTEMP(mkostemps, (char *path, int suffix, int flags), suffix, suffix, flags)

static char *dc_mkdtemp(char *path) {
    char mapped[PATH_MAX];
    const char *target = DC_PATH;
    if (target == path) return mkdtemp(path);
    if (target == dc_long_path) { errno = ENAMETOOLONG; return NULL; }
    if (!mkdtemp(mapped)) return NULL;
    dc_copy_template(path, mapped, 0);
    return path;
}

static const struct sockaddr *dc_map_socket(const struct sockaddr *addr, socklen_t *len,
                                            struct sockaddr_un *out) {
    const size_t offset = offsetof(struct sockaddr_un, sun_path);
    if (!addr || *len <= offset || addr->sa_family != AF_UNIX) return addr;
    char path[sizeof out->sun_path + 1] = {0}, mapped[PATH_MAX];
    size_t n = *len - offset;
    if (n > sizeof out->sun_path) n = sizeof out->sun_path;
    memcpy(path, (const char *)addr + offset, n);
    const char *target = DC_PATH;
    if (target == path) return addr;
    if (strlen(target) >= sizeof out->sun_path) { errno = ENAMETOOLONG; return NULL; }
    out->sun_family = AF_UNIX;
    strcpy(out->sun_path, target);
    *len = SUN_LEN(out);
    out->sun_len = *len;
    return (const struct sockaddr *)out;
}

#define DC_WRAP_SOCKET(name)                                                    \
    static int dc_##name(int fd, const struct sockaddr *addr, socklen_t len) {  \
        struct sockaddr_un out = {0};                                          \
        const struct sockaddr *mapped = dc_map_socket(addr, &len, &out);        \
        return mapped ? name(fd, mapped, len) : -1;                             \
    }

DC_WRAP_SOCKET(bind)
DC_WRAP_SOCKET(connect)
