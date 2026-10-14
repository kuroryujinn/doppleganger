/* Trusted live-test executable only. Never packaged into the production image. */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <unistd.h>

static void frame(unsigned char tag, const char *body, size_t bytes) {
    unsigned char header[9] = {tag};
    for (int i = 0; i < 8; i++) header[8-i] = (bytes >> (8*i)) & 255;
    if (write(STDOUT_FILENO, header, 9) != 9) _exit(10);
    if (bytes && write(STDOUT_FILENO, body, bytes) != (ssize_t)bytes) _exit(10);
}

int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--probe")) {
        char *args[] = {"/opt/dg-sandbox-real", "--probe", NULL};
        execv(args[0], args);
        return 10;
    }
    if (argc != 1 || getuid() != 10001) return 10;
    char input[8192] = {0};
    int fd = open("/submission/archive.zip", O_RDONLY);
    if (fd < 0 || read(fd, input, sizeof(input)-1) < 1) return 10;
    close(fd);
    if (write(STDOUT_FILENO, "DGJOB001", 8) != 8) return 10;
    frame(1, NULL, 0);
    if (input[0] == 'H') {
        fd = open("/work/private-marker", O_WRONLY|O_CREAT, 0600);
        if (fd < 0 || write(fd, "private", 7) != 7) return 10;
        close(fd);
        for (;;) pause();
    }
    if (input[0] == 'M') {
        fputs("MEMORY_STRESS_STARTED\n", stderr); fflush(stderr);
        for (;;) {
            volatile unsigned char *memory = malloc(16*1024*1024);
            if (!memory) return 11;
            for (size_t i = 0; i < 16*1024*1024; i += 4096) memory[i] = 1;
        }
    }
    if (input[0] == 'P') {
        int children = 0;
        for (;;) {
            pid_t pid = fork();
            if (pid == 0) { for (;;) pause(); }
            if (pid < 0) break;
            children++;
        }
        if (errno != EAGAIN || children >= 128) return 10;
        fprintf(stderr, "PIDS_LIMIT_ENFORCED children=%d\n", children); fflush(stderr);
        for (;;) pause();
    }
    if (input[0] == 'F') {
        signal(SIGXFSZ, SIG_IGN);
        fd = open("/work/oversize", O_WRONLY|O_CREAT, 0600);
        if (fd < 0) return 10;
        static char buffer[64*1024];
        size_t bytes = 0;
        ssize_t count;
        while ((count = write(fd, buffer, sizeof(buffer))) > 0) bytes += count;
        if (errno != EFBIG || bytes != 128*1024*1024) return 10;
        fputs("FILE_LIMIT_ENFORCED\n", stderr); fflush(stderr);
        return 1;
    }
    if (input[0] == 'I') {
        if (access("/work/private-marker", F_OK) == 0 ||
            access("/run/user/1000/docker.sock", F_OK) == 0 ||
            access(input+1, F_OK) == 0) return 10;
        fd = open("/submission/archive.zip", O_WRONLY);
        if (fd >= 0) return 10;
        fputs("CROSS_JOB_READS_AND_INPUT_WRITES_DENIED\n", stderr); fflush(stderr);
        return 1;
    }
    if (input[0] == 'X') {
        frame(2, NULL, 0);
        const char failed_exit[] = {1};
        frame(3, failed_exit, 1);
        const char report[] = "{\"schema_version\":1,\"engine_version\":\"0.1.0\",\"workflow_version\":1,\"result\":\"PASS\",\"exit_code\":1}";
        frame(16, report, sizeof(report)-1);
        frame(255, NULL, 0);
        return 0;
    }
    return 10;
}
