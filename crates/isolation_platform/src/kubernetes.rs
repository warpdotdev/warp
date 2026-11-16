/// Detect whether we are running inside a Kubernetes pod.
///
/// The kubelet unconditionally injects `KUBERNETES_SERVICE_HOST` into every pod.
pub fn is_in_kubernetes() -> bool {
    std::env::var("KUBERNETES_SERVICE_HOST").is_ok_and(|v| !v.is_empty())
}
