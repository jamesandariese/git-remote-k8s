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

## Prerequisites

You must have access to a kubernetes cluster using kubectl where you can provision a
persistent volume somehow and bind it to a claim.  If you can "create a new PVC for a pod",
you're probably good to go.  If you instead get a pending claim forever, you will need
to setup a volume provider which can do dynamic provisioning.  I've been using longhorn
lately since it has very simple backups.  I have been very happy with it but YMMV.  k3s
also comes with a local dynamic provisioner and that would also work.

## WARNING

If you use this with minikube or similar and then delete your cluster, you will lose the
git repo stored in the volume!  Use with caution in non-prod environments.  And use caution
in prod, too!  Don't forget backups.

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
