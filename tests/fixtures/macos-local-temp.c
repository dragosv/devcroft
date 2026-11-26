/* Exercise the compiled dylib from a real sandbox session. */
#include <assert.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <pwd.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <sys/xattr.h>
#include <unistd.h>
#include <utime.h>

#define OK(call) do { if ((call) < 0) { perror(#call); return 1; } } while (0)

extern char **environ;

int main(int argc, char **argv) {
    const char *temp = getenv("TMPDIR");
    assert(temp);
    if (argc > 1 && !strcmp(argv[1], "plain")) {
        errno = 0;
        assert(open("/tmp/library-test", O_CREAT | O_WRONLY, 0600) < 0 && errno == EPERM);
        char path[1024];
        snprintf(path, sizeof path, "%s/library-test", temp);
        OK(close(open(path, O_CREAT | O_WRONLY, 0600)));
        OK(unlink(path));
        return 0;
    }
    if (argc > 1) {
        assert(write(STDOUT_FILENO, "spawned", 7) == 7);
        return 0;
    }
    const char *home = getenv("HOME"), *user = getenv("USER");
    struct passwd pwd, *result = NULL;
    char pw_buffer[8192];
    assert(!strcmp(getpwuid(getuid())->pw_dir, home));
    assert(!strcmp(getpwnam(user)->pw_name, user));
    assert(getpwuid_r(getuid(), &pwd, pw_buffer, sizeof pw_buffer, &result) == 0 && result == &pwd);
    assert(!strcmp(result->pw_dir, home) && !strcmp(result->pw_shell, getenv("SHELL")));
    assert(getpwnam_r(user, &pwd, pw_buffer, 1, &result) == ERANGE && !result);
    assert(getpwnam_r(user, &pwd, pw_buffer, sizeof pw_buffer, &result) == 0 && result == &pwd);
    assert(result->pw_uid == getuid());

    OK(mkdir("/tmp/probes", 0700));
    const char *file = "/private/tmp/probes/file";
    const int dir = open("/tmp/probes", O_RDONLY | O_DIRECTORY);
    OK(dir);
    OK(mkdirat(dir, "/tmp/probes/sub", 0700));
    int fd = openat(dir, file, O_CREAT | O_WRONLY, 0600);
    OK(fd);
    OK(close(fd));
    OK(close(creat(file, 0600)));
    OK(access(file, R_OK));
    OK(faccessat(dir, file, R_OK, 0));
    OK(chmod(file, 0600));
    OK(lchmod(file, 0600));
    OK(fchmodat(dir, file, 0600, 0));
    OK(chown(file, getuid(), getgid()));
    OK(lchown(file, getuid(), getgid()));
    OK(fchownat(dir, file, getuid(), getgid(), 0));
    OK(utime(file, NULL));
    OK(utimes(file, NULL));
    OK(lutimes(file, NULL));
    OK(utimensat(dir, file, NULL, 0));
    OK(setxattr(file, "org.devcroft.temp", "x", 1, 0, 0));
    char attr[128];
    assert(getxattr(file, "org.devcroft.temp", attr, sizeof attr, 0, 0) == 1 && attr[0] == 'x');
    OK(listxattr(file, attr, sizeof attr, 0));
    OK(removexattr(file, "org.devcroft.temp", 0));
    OK(truncate(file, 5));
    struct stat st;
    OK(stat(file, &st));
    assert(st.st_size == 5);
    OK(lstat(file, &st));
    OK(fstatat(dir, file, &st, 0));
    char actual[1024], target[1024];
    snprintf(target, sizeof target, "%s/probes/file", temp);
    assert(realpath(file, actual) && !strcmp(actual, target));
    OK(link(file, "/tmp/probes/hard"));
    OK(linkat(dir, "/tmp/probes/hard", dir, "/private/tmp/probes/hard2", 0));
    OK(symlink(file, "/tmp/probes/link"));
    OK(symlinkat(file, dir, "/tmp/probes/link2"));
    assert(readlink("/tmp/probes/link", actual, sizeof actual) == (ssize_t)strlen(target));
    assert(readlinkat(dir, "/tmp/probes/link2", actual, sizeof actual) == (ssize_t)strlen(target));
    OK(rename("/tmp/probes/link", "/private/tmp/probes/renamed"));
    OK(renameat(dir, "/tmp/probes/link2", dir, "/tmp/probes/renamed2"));
    OK(renameatx_np(dir, "/tmp/probes/renamed2", dir, "/tmp/probes/renamed3", 0));
    FILE *stream = fopen("/tmp/probes/renamed", "r");
    assert(stream && freopen("/tmp/probes/renamed3", "r", stream));
    OK(fclose(stream));
    DIR *listing = opendir("/tmp/probes");
    assert(listing);
    OK(closedir(listing));
    OK(chdir("/tmp/probes"));
    assert(getcwd(actual, sizeof actual) && !strncmp(actual, temp, strlen(temp)));
    OK(unlink("hard"));
    OK(unlinkat(dir, "/tmp/probes/hard2", 0));
    OK(rmdir("/tmp/probes/sub"));
    OK(close(dir));

    char files[][64] = {"/tmp/a-XXXXXX", "/tmp/b-XXXXXX.txt", "/tmp/c-XXXXXX",
                        "/tmp/d-XXXXXX.txt", "/tmp/e-XXXXXX"};
    OK(close(mkstemp(files[0])));
    OK(close(mkstemps(files[1], 4)));
    OK(close(mkostemp(files[2], O_CLOEXEC)));
    OK(close(mkostemps(files[3], 4, O_CLOEXEC)));
    assert(mkdtemp(files[4]));
    for (int i = 0; i < 5; i++) OK(stat(files[i], &st));

    struct sockaddr_un addr = {.sun_family = AF_UNIX};
    strcpy(addr.sun_path, "/tmp/sock");
    addr.sun_len = SUN_LEN(&addr);
    int server = socket(AF_UNIX, SOCK_STREAM, 0), client = socket(AF_UNIX, SOCK_STREAM, 0);
    OK(server);
    OK(client);
    OK(bind(server, (const struct sockaddr *)&addr, addr.sun_len));
    OK(listen(server, 1));
    OK(connect(client, (const struct sockaddr *)&addr, addr.sun_len));
    memset(addr.sun_path + 5, 'x', 95);
    addr.sun_len = SUN_LEN(&addr);
    assert(bind(client, (const struct sockaddr *)&addr, addr.sun_len) == -1 && errno == ENAMETOOLONG);
    assert(connect(client, (const struct sockaddr *)&addr, addr.sun_len) == -1 && errno == ENAMETOOLONG);
    OK(close(server));
    OK(close(client));

    posix_spawn_file_actions_t actions;
    assert(posix_spawn_file_actions_init(&actions) == 0);
    assert(posix_spawn_file_actions_addopen(&actions, STDOUT_FILENO, "/tmp/subprocess",
                                            O_CREAT | O_WRONLY | O_TRUNC, 0600) == 0);
    assert(posix_spawn_file_actions_addchdir_np(&actions, "/tmp/probes") == 0);
    char *args[] = {argv[0], "child", NULL};
    pid_t pid;
    assert(posix_spawn(&pid, "/tmp/temp-probe", &actions, NULL, args, environ) == 0);
    OK(posix_spawn_file_actions_destroy(&actions));
    int status;
    OK(waitpid(pid, &status, 0));
    assert(WIFEXITED(status) && WEXITSTATUS(status) == 0);
    OK(stat("/tmp/subprocess", &st));
    assert(st.st_size == 7);

    char temp_env[2048];
    snprintf(temp_env, sizeof temp_env, "TMPDIR=%s", temp);
    char *plain_env[] = {temp_env, NULL}, *plain_args[] = {argv[0], "plain", NULL};
    OK(execve("/tmp/temp-probe", plain_args, plain_env));
    return 1;
}
