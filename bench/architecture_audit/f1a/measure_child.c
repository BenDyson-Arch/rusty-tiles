/* Linux wait4 launcher. Fork only after exec into this small process: otherwise
 * the Python fixture generator's inherited RSS contaminates the child maximum. */
#define _DEFAULT_SOURCE
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc < 3) return 125;
    pid_t pid = fork();
    if (pid == -1) return 125;
    if (pid == 0) {
        execv(argv[2], argv + 2);
        perror("exec converter");
        _exit(125);
    }
    int status;
    struct rusage usage;
    while (wait4(pid, &status, 0, &usage) == -1) {
        if (errno != EINTR) return 125;
    }
    FILE *out = fopen(argv[1], "w");
    if (!out) return 125;
    fprintf(out, "%ld\n", usage.ru_maxrss);
    if (fclose(out) != 0) return 125;
    return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}
