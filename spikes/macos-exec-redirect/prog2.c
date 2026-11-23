/* Reconstructed 2026-11-23 from the strings of the 2026-11-22 binary
 * (`spawn err %d`); the source was not kept. posix_spawn(argv[1]), so a
 * `#!` script reaches the kernel's shebang handling, and waits for it. */
#include <spawn.h>
#include <stdio.h>
#include <sys/wait.h>

extern char **environ;

int main(int argc, char **argv) {
  if (argc < 2) return 2;
  pid_t pid;
  int err = posix_spawn(&pid, argv[1], NULL, NULL, argv + 1, environ);
  if (err) {
    printf("spawn err %d\n", err);
    return 1;
  }
  int status = 0;
  waitpid(pid, &status, 0);
  return WIFEXITED(status) ? WEXITSTATUS(status) : 1;
}
