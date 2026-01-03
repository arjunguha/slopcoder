# Slopcoder

Slopcoder is a web-based frontend to coding agents that run on your local
server. If you secure access to your server, you get the benefits of web-based
access to agents, without the headache of configuring isolated environments on
third-party infrastructure.

However, Slopcoder relies on git worktrees. If you don't know how to use them,
you won't be able to use Slopcoder.

## Building from Source

Slopcoder relies on TypeScript and Rust. The simplest way to build it from
source is to run `make all`.

## Using Slopcoder

Slopcoder makes some assumptions about how you organize local copies of
your repositories. For a repository R, it assumes you have a dedicated
directory, where R/bare is a bare clone of the repository and R/X, R/Y, etc.
are worktrees. Slopcoder will create a file R/tasks.yaml that tracks the
state of its agents, as well as .jsonl files in R that hold the agents' logs.

You should not modify the .jsonl files or tasks.yaml. However, you are free to
directly update, or even delete, each worktree. When you create a new task from
the Slopcoder web interface, it creates a new feature branch and runs the agent
in a new worktree. When the agent is done, it is up to you use the command-line
on your local machine to merge the changes into another branch, make your own
changes, or discard the branch entirely.

All you need to do is create a YAML file that specifies where these directories
are. Here is an example:

```yaml
environments:
  - name: "MultiPL-E"
    directory: "/scratch/arjun-nosudo/repos/nuprl/MultiPL-E"
```

And here are the contents of that directory:

```
drwxrwx---+  8 arjun-nosudo arjun-nosudo  4096 Jan  3 09:01 bare
drwxrwx---+ 14 arjun-nosudo arjun-nosudo  4096 Jan  3 09:01 remove_logprobs
drwxrwx---+ 14 arjun-nosudo arjun-nosudo  4096 Jan  3 09:33 remove_logprobs2
-rw-rw----+  1 arjun-nosudo arjun-nosudo 17115 Jan  3 09:34 task-3d01c34c-e08b-4117-a4bd-b05aa1699e27.jsonl
-rw-rw----+  1 arjun-nosudo arjun-nosudo 24563 Jan  3 09:22 task-a5143a82-2688-451b-aaa9-d08ca281ed00.jsonl
-rw-rw----+  1 arjun-nosudo arjun-nosudo  1113 Jan  3 09:34 tasks.yaml
```

The subdirectory bare is the bare clone, and the two other directories are
worktrees that Slopcoder created.

You can start Slopcoder like this:

```bash
slopcoder-server environments.yaml --addr 127.0.0.1:8080
```

## Securing Slopcoder

Slopcoder runs agents with all guardrails off, and in a shared execution
environment. Not only can the agents see and modify each others' work, but they
can also see the Slopcoder process. Moreover, Slopcoder does not have any
authentication.

I personally run Slopcoder as an unprivileged user that is not in sudoers. I
bind it to an IP address on a Wireguard VPN that I run, and configure the
firewall so that it is only accessible from the devices that I control. If you
don't understand what this paragraph means, you probably shouldn't use
Slopcoder.
