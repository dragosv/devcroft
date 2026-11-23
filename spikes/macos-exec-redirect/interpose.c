#include <spawn.h>
#include <unistd.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>

static const char *map(const char *p, char *buf, size_t n) {
  const char *rt = getenv("DC_RT");
  if (!rt || !p) return p;
  const char *base = NULL;
  if (!strcmp(p, "/bin/sh")) base = "sh";
  else if (!strcmp(p, "/usr/bin/env")) base = "env";
  else if (!strncmp(p, "/bin/", 5)) base = p + 5;
  if (!base) return p;
  snprintf(buf, n, "%s/%s", rt, base);
  if (access(buf, X_OK) != 0) return p;
  fprintf(stderr, "[interpose] %s -> %s\n", p, buf);
  return buf;
}

static char **shebang(const char *path, char *const argv[], char *ibuf, size_t in,
                      char *mbuf, size_t mn, const char **newpath) {
  int fd = open(path, O_RDONLY);
  if (fd < 0) return NULL;
  char head[256];
  ssize_t r = read(fd, head, sizeof head - 1);
  close(fd);
  if (r < 3 || head[0] != '#' || head[1] != '!') return NULL;
  head[r] = 0;
  char *nl = strchr(head, '\n'); if (nl) *nl = 0;
  char *s = head + 2; while (*s == ' ' || *s == '\t') s++;
  char *arg = s; while (*arg && *arg != ' ' && *arg != '\t') arg++;
  if (*arg) { *arg++ = 0; while (*arg == ' ' || *arg == '\t') arg++; }
  snprintf(ibuf, in, "%s", s);
  const char *m = map(ibuf, mbuf, mn);
  if (m == ibuf) return NULL;
  int argc = 0; while (argv && argv[argc]) argc++;
  char **nv = calloc(argc + 4, sizeof *nv);
  int i = 0;
  nv[i++] = ibuf;
  if (*arg) nv[i++] = strdup(arg);
  nv[i++] = (char *)path;
  for (int k = 1; k < argc; k++) nv[i++] = argv[k];
  *newpath = m;
  return nv;
}

int my_posix_spawn(pid_t *pid, const char *path, const posix_spawn_file_actions_t *fa,
                   const posix_spawnattr_t *at, char *const argv[], char *const envp[]) {
  char b[1024], ib[256], mb[1024]; const char *np;
  char **nv = shebang(path, argv, ib, sizeof ib, mb, sizeof mb, &np);
  if (nv) return posix_spawn(pid, np, fa, at, nv, envp);
  return posix_spawn(pid, map(path, b, sizeof b), fa, at, argv, envp);
}

int my_execve(const char *path, char *const argv[], char *const envp[]) {
  char b[1024], ib[256], mb[1024]; const char *np;
  char **nv = shebang(path, argv, ib, sizeof ib, mb, sizeof mb, &np);
  if (nv) return execve(np, nv, envp);
  return execve(map(path, b, sizeof b), argv, envp);
}

__attribute__((used)) static struct { const void *n, *o; } interposers[]
    __attribute__((section("__DATA,__interpose"))) = {
  { (const void *)my_posix_spawn, (const void *)posix_spawn },
  { (const void *)my_execve, (const void *)execve },
};
