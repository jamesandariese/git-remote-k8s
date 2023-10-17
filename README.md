# `git-remote-k8s`

A Git remote helper which allows the use of a Kubernetes PVC as the
git repo storage.

## Installation

Assuming you have `$HOME/bin` in your `$PATH`, you may use the following command to install
`git-remote-k8s`.  It must be named `git-remote-k8s` (without the .sh), be executable, and
be in your PATH to work.

```bash
ln -sf $PWD/git-remote-k8s.sh $HOME/bin/git-remote-k8s
```

## Usage

The URL format for `git-remote-k8s` is `k8s://kubectl-context/namespace/pvc`.

A common name for a `kubectl` context is `default`.  Yours will be found via
`kubectl config view` and some information may be found in the
[`kubectl` Cheat Sheet](https://kubernetes.io/docs/reference/kubectl/cheatsheet/#kubectl-context-and-configuration).

For example, this PVC in the cluster accessed via the `default` context would
be `k8s://default/git-remote-test/git-remote-k8s`:
```
NAMESPACE          NAME                                                                                                             STATUS        VOLUME                                     CAPACITY   ACCESS MODES   STORAGECLASS   AGE
git-remote-test    git-remote-k8s                                                                                                   Bound         pvc-abcdef10-1023-1025-1024-123456789123   1Gi        RWO            ssd            91m
```

Cloning:

```bash
git clone k8s://context/namespace/pvc
```

Adding a remote:

```bash
git remote add origin k8s//context/namespace/pvc
```

