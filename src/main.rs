use std::process::ExitCode;
use std::{future::Future, collections::BTreeMap};
use std::sync::Arc;
use futures::{StreamExt,TryStreamExt};
use once_cell::sync::Lazy;

use anyhow::{bail, anyhow, Result};
use thiserror::Error as ThisError;

use clap::{Parser as ClapParser, ArgAction};
use tracing::{info, error, debug, trace, Level, Instrument, error_span};
use tracing_subscriber::FmtSubscriber;

use k8s::api::core::v1::*;
use k8s_openapi as k8s;
use kube::{api::*, config::KubeConfigOptions};

use tokio::task::JoinHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt, AsyncWrite, AsyncRead};


const SCHEME: &str = "k8s";
const PAT_SCHEME: &str = r"[a-zA-Z][a-zA-Z0-9+.-]*";
const PAT_PATH: &str = r"[0-9a-zA-Z](?:[0-9a-zA-Z.-]*[0-9a-zA-Z])?";
static REMOTE_PATTERN: Lazy<regex::Regex> = Lazy::new(|| {regex::Regex::new(&format!("^(?P<scheme_prefix>(?P<scheme>{PAT_SCHEME}):)?(?:/*)(?P<context>{PAT_PATH})?/(?P<namespace>{PAT_PATH})?/(?P<pvc>{PAT_PATH})?(?P<trailing>/)?")).expect("regex failed to compile")});

#[cfg(test)]
mod test;

#[derive(ClapParser, Debug, Clone)]
struct Config {
    #[arg(
        short,
        long,
        env = "GIT_REMOTE_K8S_IMAGE",
        default_value = "alpine/git:latest"
    )]
    /// Docker image used for git Jobs
    image: String,

    #[arg(
        index=1
    )]
    /// remote name
    remote_name: String,

    #[arg(
        index=2
    )]
    /// remote URL
    remote_url: String,

    #[arg(
        short,
        long,
        env = "GIT_REMOTE_K8S_DEBUG",
        action=ArgAction::Count,
    )]
    /// verbosity, may be specified more than once
    verbose: u8,
}

#[derive(ThisError,Debug)]
pub enum ApplicationError {
    /// cluster state problems
    #[error("cluster is in an inconsistent state")]
    RemoteClusterInconsistent,

    /// pod state problems
    #[error("pod metadata doesn't contain a name")]
    PodNoName,
    #[error("pod metadata doesn't contain a namespace")]
    PodNoNamespace,
    #[error("couldn't open pod's stdin")]
    PodCouldNotOpenStdin,
    #[error("couldn't open pod's stdout")]
    PodCouldNotOpenStdout,
    #[error("couldn't open pod's stderr")]
    PodCouldNotOpenStderr,
}

#[derive(ThisError,Debug)]
pub enum ConfigError {
    #[error("no namespace present in remote URL")]
    RemoteNoNamespace,
    #[error("trailing elements after pvc in remote URL")]
    RemoteTrailingElements,
    #[error("no context present in remote URL")]
    RemoteNoContext,
    #[error("no PVC name present in remote URL")]
    RemoteNoPVC,
    #[error("invalid remote URL")]
    RemoteInvalid,
    #[error("remote URL has an invalid (or no) scheme")]
    RemoteInvalidScheme,
}

impl Config {
    /// parse and validate a k8s remote pvc short-URL into a triple of Strings of the form (context, namespace, pvc)
    /// 
    /// this utilizes a regex instead of url::Url to ensure that it returns sensible errors
    // TODO: find a way to memoize this cleanly.  probably give it access to a memoizing context from AppContext.
    fn parse_and_validate(&self) -> Result<(String,String,String)> {
        let caps = REMOTE_PATTERN.captures(&self.remote_url).ok_or(ConfigError::RemoteInvalid)?;
        let scheme = if caps.name("scheme_prefix").is_none() {
            SCHEME
        } else {
            caps.name("scheme").ok_or(ConfigError::RemoteInvalidScheme)?.as_str()
        };
        if scheme != SCHEME {
            bail!(ConfigError::RemoteInvalidScheme);
        }
        let kctx = caps.name("context").ok_or(ConfigError::RemoteNoContext)?.as_str();
        let ns = caps.name("namespace").ok_or(ConfigError::RemoteNoNamespace)?.as_str();
        let pvc = caps.name("pvc").ok_or(ConfigError::RemoteNoPVC)?.as_str();
        // regex::Regex::find(REMOTE_PATTERN);
        if kctx == "" {
            bail!(ConfigError::RemoteNoContext);
        };
        if ns == "" {
            bail!(ConfigError::RemoteNoNamespace);
        };
        if pvc == "" {
            bail!(ConfigError::RemoteNoPVC);
        };
        if let Some(trailing) = caps.name("trailing") {
            if trailing.as_str() != "" {
                bail!(ConfigError::RemoteTrailingElements);
            }
        }
        Ok((kctx.to_owned(), ns.to_owned(), pvc.to_owned()))
    }

    fn get_remote_context(&self) -> Result<String> {
        let (r,_,_) = self.parse_and_validate()?;
        Ok(r)
    }
    
    fn get_remote_namespace(&self) -> Result<String> {
        let (_,r,_) = self.parse_and_validate()?;
        Ok(r)
    }

    fn get_remote_pvc(&self) -> Result<String> {
        let (_,_,r) = self.parse_and_validate()?;
        Ok(r)
    }
}

struct AppContext {
    config: Arc<Config>,
    ensures: Vec<JoinHandle<()>>,
}

impl AppContext {
    fn new() -> Result<Self> {
        Ok(Self {
            config: Arc::new(Config::parse()),
            ensures: vec![],
            // ensurance: ensurance,
        })
    }
    fn cfg(&self) -> Arc<Config> {
        self.config.clone()
    }
    async fn ktx(&self, context_name: Option<String>) -> Result<kube::Client> {
        let mut kco = KubeConfigOptions::default();
        kco.context = context_name;
        Ok(kube::Client::try_from(kube::Config::from_kubeconfig(&kco).await?)?)
    }
    fn ensure<F>(&mut self, f: F) -> EnsureHandle
      where F: Future<Output = ()> + Send + 'static
    {
        let handle = tokio::runtime::Handle::current();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.ensures.push(handle.spawn(async move {
            let _ = rx.await.unwrap_or_default(); // it's either unit or unit!  woo
            trace!("ensure handler unblocked");
            f.await;
        }));
        tx
    }
}

type EnsureHandle = tokio::sync::oneshot::Sender<()>;

impl Drop for AppContext {
    fn drop(&mut self) {
        let handle = tokio::runtime::Handle::current();
        futures::executor::block_on(async {
            for ensure in self.ensures.drain(..) {
                if let Err(e) = handle.spawn(
                    async move {
                        let _ = ensure.await.unwrap_or_default();
                        }).await {
                    eprintln!("failed to ensure in Context: {e}");
                }
            };
        });
    }
}

trait PodExt {
    fn label_selectors(&self) -> Vec<String>;
    fn field_selectors(&self) -> Result<Vec<String>>;
}

impl PodExt for Pod {
    fn label_selectors(&self) -> Vec<String> {
        let l = self.labels();
        let mut selectors = Vec::with_capacity(l.len());
        for (k,v) in l.iter() {
            format!("{}={}", k, v);
        };
        selectors
    }

    fn field_selectors(&self) -> Result<Vec<String>> {
        Ok(vec![
            format!("metadata.name={}", self.meta().name.as_ref().ok_or(ApplicationError::PodNoName)?),
            format!("metadata.namespace={}", self.meta().namespace.as_ref().ok_or(ApplicationError::PodNoNamespace)?),
        ])
    }

}

async fn wait_for_pod_running_watch(pods: &Api<Pod>, pod: Pod) -> Result<()> {
    let mut wp = WatchParams::default();
    for fs in pod.field_selectors()? {
        wp = wp.fields(&fs);
    }
    let mut stream = pods.watch(&wp, "0").await?.boxed();
    while let Some(status) = stream.try_next().await? {
        match status {
            WatchEvent::Modified(o) => {
                let s = o.status.as_ref().expect("status exists on pod");
                if s.phase.clone().unwrap_or_default() == "Running" {
                    info!("Ready to attach to {}", o.name_any());
                    break;
                }
            }
            _ => {}
        }
    };
    Ok(())
}

async fn is_pod_running(pods: &Api<Pod>, pod: Pod) -> Result<bool> {
    let got_pod = pods.get(&pod.metadata.name.ok_or(anyhow!("pod metadata must have a name"))?).await?;
    let phase = got_pod
        .status.ok_or(anyhow!("pod has no status"))?
        .phase.ok_or(anyhow!("pod has no status.phase"))?;
    if phase == "Running" {
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn wait_for_pod_running(pods: &Api<Pod>, pod: Pod) -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let xtx = tx.clone();
    let xpods = pods.clone();
    let xpod = pod.clone();
    let _p: JoinHandle<Result<()>> = tokio::spawn(async move {
        let r = is_pod_running(&xpods, xpod).await;
        if let Ok(true) = r {
            xtx.send(Ok(())).await.expect("couldn't send to channel");
        }
        if let Err(e) = r {
            xtx.send(Err(e)).await.expect("couldn't send to channel");
        }
        Ok(())
    });
    let xtx = tx.clone();
    let xpods = pods.clone();
    let xpod = pod.clone();
    let _w = tokio::spawn(async move {
        xtx.send(wait_for_pod_running_watch(&xpods, xpod).await).await.expect("couldn't send on channel");
    });
    let r = rx.recv().await;
    if r.is_none() {
        bail!("failed to read API while waiting for pod to start");
    }
    let r = r.expect("failed to extract value after checking for None");
    r
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut rc = ExitCode::from(0);
    if let Err(e) = main_wrapped(&mut rc).await {
        error!("{}", e);
        return ExitCode::from(127);
    }
    rc
}

async fn set_log_level(level: Level) {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .finish();
        tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");
}

async fn main_wrapped(rc: &mut ExitCode) -> crate::Result<()> {
    *rc = 1.into();

    let ctx = AppContext::new()?;
    let cfg = ctx.cfg();

    set_log_level(match cfg.verbose {
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    }).await;

    if let Err(_) = std::env::var("GIT_DIR") {
        error!("Please see https://github.com/jamesandariese/git-remote-k8s for details on use.");
        bail!("not running in git");
    }

    debug!("{:?}", &cfg);


    // these should all be &str to ensure we don't accidentally fail to copy any
    // and make weird errors later.  instead of making these Strings here, we need
    // to to_owned() them everywhere they're referenced to make a copy to go in the
    // data structure.
    let __kube_ns_val = cfg.get_remote_namespace()?;
    let kube_ns = __kube_ns_val.as_str();
    let __kube_context_val = cfg.get_remote_context()?;
    let kube_context = __kube_context_val.as_str();
    let __kube_pvc_val = cfg.get_remote_pvc()?;
    let kube_pvc = __kube_pvc_val.as_str();
    let __kube_worker_name_val = format!("git-remote-worker-{kube_pvc}");
    let kube_worker_name = __kube_worker_name_val.as_str();
    let kube_image = cfg.image.as_str();
    let kube_job_label = "sleeper";
    let kube_container_name = kube_job_label;
    let kube_shell_executable = "/bin/sh";
    let kube_shell_parameters = "-c";
    let kube_shell_sleeper_command = "set -e; while true;do sleep 5;done";
    let kube_repo_mount_path = "/repo";

    let kube_pod_labels = vec![
        ("com.kn8v.git-remote-k8s/repo-name", kube_pvc),
        ("com.kn8v.git-remote-k8s/job", kube_job_label),
    ];

    info!("Remote Context:   {}", kube_context);
    info!("Remote Namespace: {}", kube_ns);
    info!("Remote PVC Name:  {}", kube_pvc);

    let client = ctx.ktx(Some(kube_context.to_owned())).await?;

    let pvcs_api = kube::Api::<PersistentVolumeClaim>::namespaced(client.clone(), &kube_ns);
    let pods_api = kube::Api::<Pod>::namespaced(client.clone(), &kube_ns);

    // TODO: create the pvc

    // create the worker pod
    let mut worker_pod = Pod::default();
    worker_pod.metadata.name = Some(kube_worker_name.to_owned());
    worker_pod.metadata.namespace = Some(kube_ns.to_owned());
    {
        let mut labels = BTreeMap::new();
        for (k,v) in kube_pod_labels.iter() {
            let kk = k.to_owned().to_owned();
            let vv = v.to_owned().to_owned();
            labels.insert(kk, vv);
        }
        worker_pod.metadata.labels = Some(labels);
    }
    {
        let mut spec = PodSpec::default();
        spec.restart_policy = Some("Never".to_owned());
        {
            let mut container = Container::default();
            container.name = kube_container_name.to_owned();
            container.command = Some(vec![
                kube_shell_executable.to_owned(),
                kube_shell_parameters.to_owned(),
                kube_shell_sleeper_command.to_owned(),
            ]);
            container.image = Some(kube_image.to_owned());
            container.working_dir = Some(kube_repo_mount_path.to_owned());
            {
                let mut volume_mount = VolumeMount::default();
                volume_mount.mount_path = kube_repo_mount_path.to_owned();
                volume_mount.name = "repo".to_owned();
                container.volume_mounts = Some(vec![volume_mount]);
            }
            spec.containers = vec![container];
        }
        {
            let mut volume = Volume::default();
            volume.name = "repo".to_owned();
            {
                let mut pvcs = PersistentVolumeClaimVolumeSource::default();
                pvcs.claim_name = kube_pvc.to_owned();
                volume.persistent_volume_claim = Some(pvcs);
            }
            spec.volumes = Some(vec![volume]);
        }
        worker_pod.spec = Some(spec);
    }

    // debug!("Pod: {:?}", worker_pod);
    let mut lp = 
        ListParams::default();
    for (k,v) in kube_pod_labels {
        lp = lp.labels(&format!("{}={}", k.to_owned(), v));
    }
    debug!("list params: {lp:#?}");

    let worker_pods = pods_api.list(&lp).await?;
    // debug!("worker_pods: {worker_pods:#?}");
    if worker_pods.items.len() > 1 {
        error!("GIT-REMOTE CLUSTER IS IN AN INCONSISTENT STATE");
        error!("Your cluster has multiple pods running which are uniquely used for this repo.");
        let mut i = 0;
        for pod in worker_pods.items.iter() {
            i += 1;
            let pn = pod.metadata.name.as_ref();
            error!("pod {i}: {:?}", pn);
        }
        error!("Cannot continue while these pods are all running.");
        bail!(ApplicationError::RemoteClusterInconsistent);
    }
    let pod;
    if worker_pods.items.len() == 0 {
        let created_pod = pods_api.create(&PostParams::default(), &worker_pod).await?;
        pod = created_pod;
    } else {
        pod = worker_pods.items.into_iter().next()
            .expect("failed to take an item from an iter which is known to have enough items");
    }

    wait_for_pod_running(&pods_api, pod).await?;

    let mut gitcommand = "1>&2 echo welcome from the git-remote-k8s worker pod ; [ -f HEAD ] || git init --bare 1>&2".to_owned();
    let mut ttyout = tokio::io::stdout();
    let mut ttyin = tokio::io::stdin();

    // tokio::spawn(async {
    //     loop {
    //         sleep(Duration::from_secs(1)).await;
    //         debug!("ping");
    //     };
    // }.instrument(error_span!("pinger")));

    let connect_cmd = negotiate_git_protocol(&mut ttyout, &mut ttyin).await?
        .ok_or(anyhow!("no connect command specified and we don't know how to do anything else"))?;

    gitcommand.push_str(&format!(";echo;{connect_cmd} .;RC=$?;1>&2 echo remote worker exiting;exit $RC"));
    let ap =
        AttachParams::default()
        .stdin(true)
        .stdout(true)
        .stderr(true)
        .container(kube_container_name.to_owned());
    // let (ready_tx, ready_rx) = oneshot::channel::<()>();
    let mut stuff =pods_api.exec(kube_worker_name, vec!["sh", "-c", &gitcommand], &ap).await?;
    let mut podout = stuff.stdout().ok_or(ApplicationError::PodCouldNotOpenStdout)?;
    let mut podin = stuff.stdin().ok_or(ApplicationError::PodCouldNotOpenStdin)?;
    // pod stderr is handled specially
    let poderr = stuff.stderr().ok_or(ApplicationError::PodCouldNotOpenStderr)?;
    let mut poderr = tokio_util::io::ReaderStream::new(poderr);
    // ready_tx.send(()).expect("failed to send ready check");

    let barrier = Arc::new(tokio::sync::Barrier::new(4));

    let xbarrier: Arc<tokio::sync::Barrier> = barrier.clone();
    let _jhe = tokio::spawn(async move {
        debug!("entering");
        while let Some(l) = poderr.next().await {
            if let Err(e) = l {
                error!("error reading from pod stderr {e}");
                break;
            } else if let Ok(l) = l {
                let l = String::from_utf8_lossy(l.as_ref());
                let l = l.trim();
                info!("from pod: {l}");
            }
        }
        debug!("waiting for group to exit");
        xbarrier.wait().await;
        debug!("exiting");
    }.instrument(error_span!("pod->tty", "test" = "fred")));

    let xbarrier: Arc<tokio::sync::Barrier> = barrier.clone();
    let _jhi = tokio::spawn(async move{
        debug!("entering");
        tokio::io::copy(&mut ttyin, &mut podin).await.expect("error copying tty input to pod input");
        podin.flush().await.expect("final flush to pod failed");
        debug!("waiting for group to exit");
        xbarrier.wait().await;
        debug!("exiting");
    }.instrument(error_span!("git->pod")));
    let xbarrier: Arc<tokio::sync::Barrier> = barrier.clone();
    let _jho = tokio::spawn(async move {
        debug!("entering");
        tokio::io::copy(&mut podout, &mut ttyout).await.expect("error copying pod output to tty output");
        ttyout.flush().await.expect("final flush to git failed");
        debug!("waiting for group to exit");
        xbarrier.wait().await;
        debug!("exiting");
    }.instrument(error_span!("git<-pod")));

    let status = stuff.take_status()
        .expect("failed to take status").await
        .ok_or(anyhow!("could not take status of remote git worker"))?;
    // this is an exit code which is always nonzero.
    // we'll _default_ to 1 instead of 0 because we only return _anything_ other than 0
    // when NonZeroExitCode is also given as the exit reason.
    debug!("exit code of job: {status:#?}");
    let exitcode = (|| -> Option<u8>{
        let exitcode = status.details.as_ref()?.causes.as_ref()?.first()?.message.to_owned()?;
        if let Ok(rc) = exitcode.parse::<u8>() {
            return Some(rc);
        }
        return Some(1);
    })().unwrap_or(1);
    debug!("exit status code of remote job discovered was {exitcode:?}");
    // finally, we'll set the exit code of the entire application
    // to the exit code of the pod, if possible.  if we know it's
    // non-zero and can't figure out what it was, it will be 1.
    // if we know it's zero (because it's not non-zero), then we
    // return 0.  if we _can_ figure it out though, we use the
    // integer exit code.
    if status.reason == Some("NonZeroExitCode".to_owned()) {
        info!("exit status of remote job: {}", exitcode);
        *rc = exitcode.into();
    } else {
        *rc = 0.into();
    }
    debug!("waiting for group to exit");
    barrier.wait().await;
    let _ = stuff.join().await?;

    Ok(())
}

async fn get_tokio_line_by_bytes(podout: &mut (impl AsyncRead + Unpin)) -> Result<String> {
    let mut r = String::with_capacity(512);

    loop{
        let c = podout.read_u8().await?;
        if c == b'\n' {
            return Ok(r);
        }
        r.push(c.into())
    }
}

#[derive(ThisError,Debug)]
enum ProtocolError {
    #[error("no command given via git protocol")]
    NoCommandGiven,
    #[error("no service given to connect command")]
    NoServiceGiven,
    #[error("unknown command specified")]
    UnknownCommand(String),
}

async fn negotiate_git_protocol(ttytx: &mut (impl AsyncWrite + Unpin), ttyrx: &mut (impl AsyncRead + Unpin)) -> Result<Option<String>> {
    loop {
        let cmd = get_tokio_line_by_bytes(ttyrx).await?;
        let mut argv = cmd.split_whitespace();
        let cmd = argv.next().ok_or(ProtocolError::NoCommandGiven)?;

        match cmd {
            "capabilities" => {
                ttytx.write_all(b"connect\n\n").await?;
            },
            "connect" => {
                let service = argv.next().ok_or(ProtocolError::NoServiceGiven)?;
                return Ok(Some(service.to_owned()));
            },
            unknown => {
                return Err(anyhow!(ProtocolError::UnknownCommand(unknown.to_owned())));
            },
        }
    }
}
