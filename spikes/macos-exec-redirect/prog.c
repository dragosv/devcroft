/* Reconstructed 2026-11-23 from the strings of the 2026-11-22 binary
 * (`system(): `, the two commands, `popen(): %s`); the source was not kept. */
#include <stdio.h>
#include <stdlib.h>

int main(void) {
  printf("system(): ");
  fflush(stdout);
  int rc = system("echo via-system; echo shell-is:${BASH_VERSION:-?}");
  FILE *p = popen("echo via-popen", "r");
  char line[256] = "";
  if (p) {
    if (!fgets(line, sizeof line, p)) line[0] = 0;
    pclose(p);
  }
  printf("popen(): %s", line);
  return rc == 0 ? 0 : 1;
}
