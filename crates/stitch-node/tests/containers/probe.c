// Escape probes run inside an app container by the live container tests
// (design §2.23): each prints "REACHED <what>" when it gets through and
// "BLOCKED <what>: <errno>" when it does not. Built statically.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <sys/un.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <sys/wait.h>
#include <sys/stat.h>
#include <stddef.h>
#include <signal.h>

static void say(int ok, const char *what) {
  if (ok) printf("REACHED %s\n", what);
  else printf("BLOCKED %s: %s\n", what, strerror(errno));
  fflush(stdout);
}

static int unix_path(const char *p) {
  int s = socket(AF_UNIX, SOCK_STREAM, 0);
  if (s < 0) return 0;
  struct sockaddr_un a = {.sun_family = AF_UNIX};
  strncpy(a.sun_path, p, sizeof a.sun_path - 1);
  int r = connect(s, (struct sockaddr *)&a, sizeof a) == 0;
  close(s);
  return r;
}

static int unix_abstract(const char *p) {
  int s = socket(AF_UNIX, SOCK_STREAM, 0);
  if (s < 0) return 0;
  struct sockaddr_un a = {.sun_family = AF_UNIX};
  size_t n = strlen(p);
  memcpy(a.sun_path + 1, p, n);
  int r = connect(s, (struct sockaddr *)&a, offsetof(struct sockaddr_un, sun_path) + 1 + n) == 0;
  close(s);
  return r;
}

static int tcp(const char *ip, int port) {
  int s = socket(AF_INET, SOCK_STREAM, 0);
  if (s < 0) return 0;
  struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = htons(port)};
  inet_pton(AF_INET, ip, &a.sin_addr);
  int r = connect(s, (struct sockaddr *)&a, sizeof a) == 0;
  close(s);
  return r;
}

int main(int argc, char **argv) {
  char what[512];
  for (int i = 1; i < argc; i++) {
    const char *t = argv[i];
    if (!strncmp(t, "unix:", 5)) {
      snprintf(what, sizeof what, "%s", t);
      say(unix_path(t + 5), what);
    } else if (!strncmp(t, "abstract:", 9)) {
      snprintf(what, sizeof what, "%s", t);
      say(unix_abstract(t + 9), what);
    } else if (!strncmp(t, "tcp:", 4)) {
      char ip[64];
      int port;
      if (sscanf(t + 4, "%63[^:]:%d", ip, &port) == 2) say(tcp(ip, port), t);
    } else if (!strncmp(t, "read:", 5)) {
      int fd = open(t + 5, O_RDONLY);
      say(fd >= 0, t);
      if (fd >= 0) close(fd);
    } else if (!strncmp(t, "write:", 6)) {
      int fd = open(t + 6, O_WRONLY | O_CREAT | O_TRUNC, 0755);
      int ok = fd >= 0 && write(fd, "#!/bin/sh\necho ran\n", 19) == 19;
      say(ok, t);
      if (fd >= 0) close(fd);
    } else if (!strncmp(t, "exec:", 5)) {
      pid_t p = fork();
      if (p == 0) {
        execl(t + 5, t + 5, (char *)0);
        _exit(127);
      }
      int st = 0;
      waitpid(p, &st, 0);
      errno = EACCES;
      say(WIFEXITED(st) && WEXITSTATUS(st) == 0, t);
    } else if (!strcmp(t, "userns")) {
      say(unshare(CLONE_NEWUSER) == 0, "unshare(CLONE_NEWUSER)");
    } else if (!strcmp(t, "netns")) {
      say(unshare(CLONE_NEWNET) == 0, "unshare(CLONE_NEWNET)");
    } else if (!strcmp(t, "clone-userns")) {
      long r = syscall(SYS_clone, CLONE_NEWUSER | SIGCHLD, 0, 0, 0, 0);
      if (r == 0) _exit(0);
      say(r > 0, "clone(CLONE_NEWUSER)");
    } else if (!strcmp(t, "io_uring")) {
      char params[120] = {0};
      long r = syscall(425, 4, params);
      say(r >= 0, "io_uring_setup");
    } else if (!strcmp(t, "mount")) {
      say(syscall(SYS_mount, "none", "/tmp", "tmpfs", 0, 0) == 0, "mount");
    } else if (!strcmp(t, "keyctl")) {
      say(syscall(SYS_keyctl, 0, -3, 0, 0, 0) >= 0, "keyctl");
    } else if (!strcmp(t, "ptrace")) {
      say(syscall(SYS_ptrace, 0, 0, 0, 0) == 0, "ptrace(TRACEME)");
    } else if (!strcmp(t, "symlink")) {
      say(symlink("/etc/passwd", "/out/leak") == 0, "symlink in out");
      say(symlink("/home/app", "/out/dir") == 0, "dir symlink in out");
    } else if (!strcmp(t, "fifo")) {
      say(mkfifo("/out/pipe", 0644) == 0, "fifo in out");
    } else if (!strncmp(t, "alloc:", 6)) {
      long mb = atol(t + 6);
      for (long j = 0; j < mb; j++) {
        char *b = malloc(1 << 20);
        if (!b) break;
        memset(b, 1, 1 << 20);
      }
      say(1, t);
    } else if (!strcmp(t, "id")) {
      printf("UID %d GID %d\n", getuid(), getgid());
      fflush(stdout);
    }
  }
  return 0;
}
