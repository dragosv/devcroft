/* SPDX-License-Identifier: Apache-2.0
 * Fixed syscall probes run as a real keeper child. No host credentials, public
 * Internet peer, arbitrary pathname or network target is taken from argv.
 */
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <net/if.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <unistd.h>

static int first = 1;
static void result(const char *name, int rc, int error) {
    printf("%s{\"name\":\"%s\",\"rc\":%d,\"errno\":%d}",
           first ? "" : ",", name, rc, error);
    first = 0;
}

static int stream(const char *ip, unsigned short port, int *error) {
    int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0);
    if (fd < 0) { *error = errno; return -1; }
    struct sockaddr_in addr = { .sin_family = AF_INET, .sin_port = htons(port) };
    if (inet_pton(AF_INET, ip, &addr.sin_addr) != 1) _exit(2);
    int rc = connect(fd, (struct sockaddr *)&addr, sizeof(addr));
    *error = rc < 0 ? errno : 0;
    if (rc < 0 && *error == EINPROGRESS) {
        struct pollfd poller = { .fd = fd, .events = POLLOUT };
        if (poll(&poller, 1, 1000) > 0) {
            socklen_t size = sizeof(*error);
            if (getsockopt(fd, SOL_SOCKET, SO_ERROR, error, &size)) *error = errno;
        } else *error = ETIMEDOUT;
    }
    if (*error) { close(fd); return -1; }
    int flags = fcntl(fd, F_GETFL);
    if (flags < 0 || fcntl(fd, F_SETFL, flags & ~O_NONBLOCK)) _exit(3);
    struct timeval timeout = { .tv_sec = 1 };
    if (setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout)) ||
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout))) _exit(4);
    return fd;
}

static void connect_probe(const char *name, const char *ip, unsigned short port) {
    int error = 0, fd = stream(ip, port, &error);
    result(name, fd < 0 ? -1 : 0, error);
    if (fd >= 0) close(fd);
}

static void bind_probe(const char *name, unsigned short port) {
    int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) _exit(5);
    struct sockaddr_in addr = { .sin_family = AF_INET, .sin_port = htons(port),
                               .sin_addr.s_addr = htonl(INADDR_LOOPBACK) };
    int rc = bind(fd, (struct sockaddr *)&addr, sizeof(addr)), error = rc < 0 ? errno : 0;
    result(name, rc, error);
    close(fd);
}

static void udp_local(void) {
    int receiver = socket(AF_INET, SOCK_DGRAM | SOCK_CLOEXEC, 0);
    int sender = socket(AF_INET, SOCK_DGRAM | SOCK_CLOEXEC, 0);
    if (receiver < 0 || sender < 0) _exit(6);
    struct sockaddr_in addr = { .sin_family = AF_INET, .sin_port = htons(28081),
                               .sin_addr.s_addr = htonl(INADDR_LOOPBACK) };
    struct timeval timeout = { .tv_sec = 1 };
    if (setsockopt(receiver, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout))) _exit(7);
    int rc = bind(receiver, (struct sockaddr *)&addr, sizeof(addr)), error = rc < 0 ? errno : 0;
    const char payload[] = "boundary-udp-local";
    char reply[64];
    if (!rc) {
        if (sendto(sender, payload, sizeof(payload), 0, (struct sockaddr *)&addr, sizeof(addr)) != sizeof(payload)) {
            rc = -1; error = errno;
        } else {
            ssize_t n = recv(receiver, reply, sizeof(reply), 0);
            if (n != sizeof(payload) || memcmp(payload, reply, sizeof(payload))) {
                rc = -1; error = n < 0 ? errno : EIO;
            }
        }
    }
    result("udp_unlisted_loopback_delivery", rc, error);
    close(receiver); close(sender);
}

static void udp_outside(void) {
    int fd = socket(AF_INET, SOCK_DGRAM | SOCK_CLOEXEC, 0);
    if (fd < 0) _exit(8);
    struct sockaddr_in addr = { .sin_family = AF_INET, .sin_port = htons(53) };
    if (inet_pton(AF_INET, "192.0.2.1", &addr.sin_addr) != 1) _exit(9);
    const char payload[] = "boundary-route-test";
    int rc = (int)sendto(fd, payload, sizeof(payload), 0, (struct sockaddr *)&addr, sizeof(addr));
    result("udp_external_send", rc, rc < 0 ? errno : 0);
    close(fd);
}

static void open_probe(const char *name, const char *path, int flags) {
    errno = 0;
    int fd = open(path, flags | O_CLOEXEC, 0600), error = fd < 0 ? errno : 0;
    result(name, fd < 0 ? -1 : 0, error);
    if (fd >= 0) close(fd);
}

static void private_files(void) {
    const char *path = "/workspace/.boundary-private";
    const char *hard = "/workspace/.boundary-private-link";
    const char *moved = "/workspace/.boundary-private-moved";
    int fd = open(path, O_CREAT | O_EXCL | O_RDWR | O_CLOEXEC, 0600), ok = fd >= 0;
    char data[8];
    if (ok) {
        ok = write(fd, "private", 7) == 7 && pread(fd, data, 7, 0) == 7
             && !memcmp(data, "private", 7) && !ftruncate(fd, 3) && !fsync(fd);
        close(fd);
    }
    ok = !link(path, hard) && !rename(hard, moved) && !unlink(moved) && ok;
    if (unlink(path)) ok = 0;
    result("private_file_operations", ok ? 0 : -1, ok ? 0 : EIO);
}

int main(void) {
    alarm(15);
    int fd = socket(AF_INET, SOCK_DGRAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return 10;
    struct ifreq interfaces[16];
    struct ifconf config = { .ifc_len = sizeof(interfaces), .ifc_req = interfaces };
    if (ioctl(fd, SIOCGIFCONF, &config)) return 11;
    printf("{\"schema\":1,\"ipv4_interfaces\":[");
    int count = config.ifc_len / (int)sizeof(struct ifreq);
    if (count < 1 || count > 16) return 12;
    for (int i = 0; i < count; ++i) {
        /* Fixture interface names must be this exact known printable name. */
        if (strcmp(interfaces[i].ifr_name, "lo")) return 13;
        printf("%s\"lo\"", i ? "," : "");
    }
    close(fd);
    int v6 = socket(AF_INET6, SOCK_STREAM | SOCK_CLOEXEC, 0), v6_error = v6 < 0 ? errno : 0;
    if (v6 >= 0) close(v6);
    printf("],\"ipv6_socket_errno\":%d,\"errno_constants\":{\"EACCES\":%d,\"EROFS\":%d,"
           "\"EXDEV\":%d,\"ENETUNREACH\":%d,\"EAFNOSUPPORT\":%d,\"EIO\":%d},\"checks\":[",
           v6_error, EACCES, EROFS, EXDEV, ENETUNREACH, EAFNOSUPPORT, EIO);

    int error = 0;
    fd = stream("127.0.0.1", 8080, &error);
    char response[1024] = {0}, identity[33] = {0};
    if (fd >= 0) {
        const char query[] = "GET / HTTP/1.1\r\nHost: fixture\r\nConnection: close\r\n\r\n";
        if (send(fd, query, sizeof(query) - 1, MSG_NOSIGNAL) != sizeof(query) - 1) return 14;
        size_t used = 0;
        for (;;) {
            ssize_t n = recv(fd, response + used, sizeof(response) - used - 1, 0);
            if (n < 0) return 15;
            if (!n) break;
            used += (size_t)n;
            if (used >= sizeof(response) - 1) return 16;
        }
        char *marker = strstr(response, "instance=");
        if (marker && strlen(marker + 9) >= 32 && strspn(marker + 9, "0123456789abcdef") == 32)
            memcpy(identity, marker + 9, 32);
        close(fd);
    }
    result("tcp_allowed_http_identity", identity[0] ? 0 : -1, error);
    connect_probe("tcp_live_unlisted_port", "127.0.0.1", 8081);
    connect_probe("tcp_external_unlisted_port", "192.0.2.1", 8081);
    connect_probe("tcp_external_allowed_port", "192.0.2.1", 8080);
    bind_probe("tcp_allowed_bind", 6379);
    bind_probe("tcp_unlisted_bind", 12345);
    udp_local(); udp_outside();

    const char *cache = "/toolchain/fixture/xxHash/Makefile";
    fd = open(cache, O_RDONLY | O_CLOEXEC);
    char canary[32];
    int read_ok = fd >= 0 && read(fd, canary, sizeof(canary)) > 0;
    if (fd >= 0) close(fd);
    result("cache_existing_file_read", read_ok ? 0 : -1, read_ok ? 0 : EIO);
    private_files();
    open_probe("cache_append", cache, O_WRONLY | O_APPEND);
    open_probe("cache_truncate", cache, O_WRONLY | O_TRUNC);
    open_probe("cache_create", "/toolchain/fixture/.boundary-created", O_CREAT | O_EXCL | O_WRONLY);
    errno = 0; int rc = unlink(cache); result("cache_unlink", rc, rc < 0 ? errno : 0);
    errno = 0; rc = rename(cache, "/toolchain/fixture/xxHash/.boundary-renamed");
    result("cache_rename", rc, rc < 0 ? errno : 0);
    errno = 0; rc = link(cache, "/toolchain/fixture/xxHash/.boundary-hardlink");
    result("cache_hardlink", rc, rc < 0 ? errno : 0);
    if (symlink(cache, "/workspace/.boundary-cache-alias")) return 17;
    open_probe("cache_write_via_workspace_symlink", "/workspace/.boundary-cache-alias", O_WRONLY | O_APPEND);
    if (unlink("/workspace/.boundary-cache-alias")) return 18;
    open_probe("raw_cache_device_read", "/dev/vdc", O_RDONLY);
    open_probe("raw_private_device_read", "/dev/vdd", O_RDONLY);
    open_probe("raw_bootstrap_device_read", "/dev/vdb", O_RDONLY);
    printf("],\"http_instance_id\":\"%s\"}\n", identity);
    return 0;
}
