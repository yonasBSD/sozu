use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    convert::From,
    default::Default,
    fmt,
    net::SocketAddr,
};

use crate::{
    certificate::TlsVersion,
    config::{
        DEFAULT_CIPHER_SUITES, DEFAULT_GROUPS_LIST, DEFAULT_RUSTLS_CIPHER_LIST,
        DEFAULT_SIGNATURE_ALGORITHMS,
    },
    request::{default_sticky_name, is_false, Cluster, LoadBalancingParams, PROTOCOL_VERSION},
    state::{ClusterId, ConfigState, RouteKey},
};

/// Responses of the main process to the CLI (or other client)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub version: u8,
    pub status: ResponseStatus,
    pub message: String,
    pub content: Option<ResponseContent>,
}

impl Response {
    pub fn new(
        // id: String,
        status: ResponseStatus,
        message: String,
        content: Option<ResponseContent>,
    ) -> Response {
        Response {
            version: PROTOCOL_VERSION,
            id: "generic-response-id-to-be-removed".to_string(),
            status,
            message,
            content,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResponseStatus {
    Ok,
    Processing,
    Failure,
}

/// details of a response sent by the main process to the client
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResponseContent {
    /// a list of workers, with ids, pids, statuses
    Workers(Vec<WorkerInfo>),
    /// aggregated metrics of main process and workers
    Metrics(AggregatedMetricsData),
    /// worker responses to a same query: worker_id -> query_answer
    Query(BTreeMap<String, QueryAnswer>),
    /// the state of Sōzu: frontends, backends, listeners, etc.
    State(Box<ConfigState>),
    /// a proxy event
    Event(Event),
    /// a filtered list of frontend
    FrontendList(ListedFrontends),
    // this is new
    Status(Vec<WorkerInfo>),
    /// all listeners
    ListenersList(ListenersList),
}

/// details of an query answer, sent by a worker
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QueryAnswer {
    Clusters(Vec<QueryAnswerCluster>),
    /// cluster id -> hash of cluster information
    ClustersHashes(BTreeMap<String, u64>),
    Certificates(QueryAnswerCertificate),
    Metrics(QueryAnswerMetrics),
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueryAnswerCluster {
    pub configuration: Option<Cluster>,
    pub http_frontends: Vec<HttpFrontend>,
    pub https_frontends: Vec<HttpFrontend>,
    pub tcp_frontends: Vec<TcpFrontend>,
    pub backends: Vec<Backend>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryAnswerCertificate {
    /// returns a list of certificates: domain -> fingerprint
    All(HashMap<SocketAddr, BTreeMap<String, Vec<u8>>>),
    /// returns a fingerprint
    Domain(HashMap<SocketAddr, Option<(String, Vec<u8>)>>),
    /// returns the certificate
    Fingerprint(Option<(String, Vec<String>)>),
}

/// Returned by the local drain
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryAnswerMetrics {
    /// (list of proxy metrics, list of cluster metrics)
    List((Vec<String>, Vec<String>)),
    /// all worker metrics, proxy & clusters, with Options all around for partial answers
    All(WorkerMetrics),
    /// Use to trickle up errors to the CLI
    Error(String),
}

#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HttpFrontend {
    pub route: Route,
    pub address: SocketAddr,
    pub hostname: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "is_default_path_rule")]
    pub path: PathRule,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default)]
    pub position: RulePosition,
    pub tags: Option<BTreeMap<String, String>>,
}

impl HttpFrontend {
    /// `is_cluster_id` check if the frontend is dedicated to the given cluster_id
    pub fn is_cluster_id(&self, cluster_id: &str) -> bool {
        matches!(&self.route, Route::ClusterId(id) if id == cluster_id)
    }

    /// `route_key` returns a representation of the frontend as a route key
    pub fn route_key(&self) -> RouteKey {
        self.into()
    }
}

/// The cluster to which the traffic will be redirected
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Route {
    /// send a 401 default answer
    Deny,
    /// the cluster to which the frontend belongs
    ClusterId(ClusterId),
}

impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Route::Deny => write!(f, "deny"),
            Route::ClusterId(string) => write!(f, "{string}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RulePosition {
    Pre,
    Post,
    Tree,
}

impl Default for RulePosition {
    fn default() -> Self {
        RulePosition::Tree
    }
}

/// A filter for the path of incoming requests
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PathRule {
    /// filters paths that start with a pattern, typically "/api"
    Prefix(String),
    /// filters paths that match a regex pattern
    Regex(String),
    /// filters paths that exactly match a pattern, no more, no less
    Equals(String),
}

impl PathRule {
    pub fn from_cli_options(
        path_prefix: Option<String>,
        path_regex: Option<String>,
        path_equals: Option<String>,
    ) -> Self {
        match (path_prefix, path_regex, path_equals) {
            (Some(prefix), _, _) => PathRule::Prefix(prefix),
            (None, Some(regex), _) => PathRule::Regex(regex),
            (None, None, Some(equals)) => PathRule::Equals(equals),
            _ => PathRule::default(),
        }
    }
}

impl Default for PathRule {
    fn default() -> Self {
        PathRule::Prefix(String::new())
    }
}

fn is_default_path_rule(p: &PathRule) -> bool {
    match p {
        PathRule::Regex(_) => false,
        PathRule::Equals(_) => false,
        PathRule::Prefix(s) => s.is_empty(),
    }
}

impl std::fmt::Display for PathRule {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PathRule::Prefix(s) => write!(f, "prefix '{s}'"),
            PathRule::Regex(r) => write!(f, "regexp '{}'", r.as_str()),
            PathRule::Equals(s) => write!(f, "equals '{s}'"),
        }
    }
}

#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TcpFrontend {
    pub cluster_id: String,
    pub address: SocketAddr,
    pub tags: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ListedFrontends {
    pub http_frontends: Vec<HttpFrontend>,
    pub https_frontends: Vec<HttpFrontend>,
    pub tcp_frontends: Vec<TcpFrontend>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Backend {
    pub cluster_id: String,
    pub backend_id: String,
    pub address: SocketAddr,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticky_id: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_balancing_parameters: Option<LoadBalancingParams>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<bool>,
}

impl Ord for Backend {
    fn cmp(&self, o: &Backend) -> Ordering {
        self.cluster_id
            .cmp(&o.cluster_id)
            .then(self.backend_id.cmp(&o.backend_id))
            .then(self.sticky_id.cmp(&o.sticky_id))
            .then(
                self.load_balancing_parameters
                    .cmp(&o.load_balancing_parameters),
            )
            .then(self.backup.cmp(&o.backup))
            .then(socketaddr_cmp(&self.address, &o.address))
    }
}

impl PartialOrd for Backend {
    fn partial_cmp(&self, other: &Backend) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// All listeners, listed for the CLI.
/// the bool indicates if it is active or not
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ListenersList {
    pub http_listeners: HashMap<SocketAddr, (HttpListenerConfig, bool)>,
    pub https_listeners: HashMap<SocketAddr, (HttpsListenerConfig, bool)>,
    pub tcp_listeners: HashMap<SocketAddr, (TcpListenerConfig, bool)>,
}

/// details of an HTTP listener, sent by the main process to the worker
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HttpListenerConfig {
    pub address: SocketAddr,
    pub public_address: Option<SocketAddr>,
    pub answer_404: String,
    pub answer_503: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "is_false")]
    pub expect_proxy: bool,
    /// identifies sticky sessions
    #[serde(default = "default_sticky_name")]
    pub sticky_name: String,
    /// client inactive time
    pub front_timeout: u32,
    /// backend server inactive time
    pub back_timeout: u32,
    /// time to connect to the backend
    pub connect_timeout: u32,
    /// max time to send a complete request
    pub request_timeout: u32,
}

// TODO: set the default values elsewhere, see #873
impl Default for HttpListenerConfig {
    fn default() -> HttpListenerConfig {
        HttpListenerConfig {
            address:           "127.0.0.1:8080".parse().expect("could not parse address"),
              public_address:  None,
              answer_404:      String::from("HTTP/1.1 404 Not Found\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"),
              answer_503:      String::from("HTTP/1.1 503 Service Unavailable\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"),
              expect_proxy:    false,
              sticky_name:     String::from("SOZUBALANCEID"),
              front_timeout:   60,
              back_timeout:    30,
              connect_timeout: 3,
              request_timeout: 10,
        }
    }
}

/// details of an HTTPS listener, sent by the main process to the worker
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HttpsListenerConfig {
    pub address: SocketAddr,
    pub public_address: Option<SocketAddr>,
    pub answer_404: String,
    pub answer_503: String,
    pub versions: Vec<TlsVersion>,
    pub cipher_list: Vec<String>,
    #[serde(default)]
    pub cipher_suites: Vec<String>,
    #[serde(default)]
    pub signature_algorithms: Vec<String>,
    #[serde(default)]
    pub groups_list: Vec<String>,
    #[serde(default)]
    pub expect_proxy: bool,
    #[serde(default = "default_sticky_name")]
    pub sticky_name: String,
    #[serde(default)]
    pub certificate: Option<String>,
    #[serde(default)]
    pub certificate_chain: Vec<String>,
    #[serde(default)]
    pub key: Option<String>,
    pub front_timeout: u32,
    pub back_timeout: u32,
    pub connect_timeout: u32,
    /// max time to send a complete request
    pub request_timeout: u32,
}

impl Default for HttpsListenerConfig {
    fn default() -> HttpsListenerConfig {
        HttpsListenerConfig {
      address:         "127.0.0.1:8443".parse().expect("could not parse address"),
      public_address:  None,
      answer_404:      String::from("HTTP/1.1 404 Not Found\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"),
      answer_503:      String::from("HTTP/1.1 503 Service Unavailable\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"),
      cipher_list:     DEFAULT_RUSTLS_CIPHER_LIST.into_iter()
          .map(String::from)
          .collect(),
      cipher_suites:  DEFAULT_CIPHER_SUITES.into_iter()
          .map(String::from)
          .collect(),
      signature_algorithms: DEFAULT_SIGNATURE_ALGORITHMS.into_iter()
          .map(String::from)
          .collect(),
      groups_list: DEFAULT_GROUPS_LIST.into_iter()
          .map(String::from)
          .collect(),
      versions:            vec!(TlsVersion::TLSv1_2),
      expect_proxy:        false,
      sticky_name:         String::from("SOZUBALANCEID"),
      certificate:         None,
      certificate_chain:   vec![],
      key:                 None,
      front_timeout:   60,
      back_timeout:    30,
      connect_timeout: 3,
      request_timeout: 10,
    }
    }
}

/// details of an TCP listener, sent by the main process to the worker
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TcpListenerConfig {
    pub address: SocketAddr,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_address: Option<SocketAddr>,
    #[serde(default)]
    #[serde(skip_serializing_if = "is_false")]
    pub expect_proxy: bool,
    pub front_timeout: u32,
    pub back_timeout: u32,
    pub connect_timeout: u32,
}

/// Runstate of a worker
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunState {
    Running,
    Stopping,
    Stopped,
    NotAnswering,
}

impl fmt::Display for RunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub id: u32,
    pub pid: i32,
    pub run_state: RunState,
}

/// a backend event that happened on a proxy
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Event {
    BackendDown(String, SocketAddr),
    BackendUp(String, SocketAddr),
    NoAvailableBackends(String),
    /// indicates a backend that was removed from configuration has no lingering connections
    /// so it can be safely stopped
    RemovedBackendHasNoConnections(String, SocketAddr),
}

impl From<ProxyEvent> for Event {
    fn from(e: ProxyEvent) -> Self {
        match e {
            ProxyEvent::BackendDown(id, addr) => Event::BackendDown(id, addr),
            ProxyEvent::BackendUp(id, addr) => Event::BackendUp(id, addr),
            ProxyEvent::NoAvailableBackends(cluster_id) => Event::NoAvailableBackends(cluster_id),
            ProxyEvent::RemovedBackendHasNoConnections(id, addr) => {
                Event::RemovedBackendHasNoConnections(id, addr)
            }
        }
    }
}

#[derive(Serialize)]
struct StatePath {
    path: String,
}

pub type MessageId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyResponse {
    pub id: MessageId,
    pub status: ProxyResponseStatus,
    pub content: Option<ProxyResponseContent>,
}

impl ProxyResponse {
    pub fn ok<T>(id: T) -> Self
    where
        T: ToString,
    {
        Self {
            id: id.to_string(),
            status: ProxyResponseStatus::Ok,
            content: None,
        }
    }

    pub fn error<T, U>(id: T, error: U) -> Self
    where
        T: ToString,
        U: ToString,
    {
        Self {
            id: id.to_string(),
            status: ProxyResponseStatus::Error(error.to_string()),
            content: None,
        }
    }

    pub fn processing<T>(id: T) -> Self
    where
        T: ToString,
    {
        Self {
            id: id.to_string(),
            status: ProxyResponseStatus::Processing,
            content: None,
        }
    }

    pub fn status<T>(id: T, status: ProxyResponseStatus) -> Self
    where
        T: ToString,
    {
        Self {
            id: id.to_string(),
            status,
            content: None,
        }
    }
}

impl fmt::Display for ProxyResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}-{:?}", self.id, self.status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProxyResponseStatus {
    Ok,
    Processing,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProxyResponseContent {
    /// contains proxy & cluster metrics
    Metrics(WorkerMetrics),
    Query(QueryAnswer),
    Event(ProxyEvent),
}

/// Aggregated metrics of main process & workers, for the CLI
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatedMetricsData {
    pub main: BTreeMap<String, FilteredData>,
    pub workers: BTreeMap<String, QueryAnswer>,
}

/// All metrics of a worker: proxy and clusters
/// Populated by Options so partial results can be sent
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerMetrics {
    /// Metrics of the worker process, key -> value
    pub proxy: Option<BTreeMap<String, FilteredData>>,
    /// cluster_id -> cluster_metrics
    pub clusters: Option<BTreeMap<String, ClusterMetricsData>>,
}

/// the metrics of a given cluster, with several backends
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterMetricsData {
    /// metric name -> metric value
    pub cluster: Option<BTreeMap<String, FilteredData>>,
    /// backend_id -> (metric name-> metric value)
    pub backends: Option<BTreeMap<String, BTreeMap<String, FilteredData>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FilteredData {
    Gauge(usize),
    Count(i64),
    Time(usize),
    Percentiles(Percentiles),
    TimeSerie(FilteredTimeSerie),
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FilteredTimeSerie {
    pub last_second: u32,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub last_minute: Vec<u32>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub last_hour: Vec<u32>,
}

impl fmt::Debug for FilteredTimeSerie {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "FilteredTimeSerie {{\nlast_second: {},\nlast_minute:\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\nlast_hour:\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n}}",
      self.last_second,
    &self.last_minute[0..10], &self.last_minute[10..20], &self.last_minute[20..30], &self.last_minute[30..40], &self.last_minute[40..50], &self.last_minute[50..60],
    &self.last_hour[0..10], &self.last_hour[10..20], &self.last_hour[20..30], &self.last_hour[30..40], &self.last_hour[40..50], &self.last_hour[50..60])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Percentiles {
    pub samples: u64,
    pub p_50: u64,
    pub p_90: u64,
    pub p_99: u64,
    pub p_99_9: u64,
    pub p_99_99: u64,
    pub p_99_999: u64,
    pub p_100: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendMetricsData {
    pub bytes_in: usize,
    pub bytes_out: usize,
    pub percentiles: Percentiles,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProxyEvent {
    BackendDown(String, SocketAddr),
    BackendUp(String, SocketAddr),
    NoAvailableBackends(String),
    RemovedBackendHasNoConnections(String, SocketAddr),
}

fn socketaddr_cmp(a: &SocketAddr, b: &SocketAddr) -> Ordering {
    a.ip().cmp(&b.ip()).then(a.port().cmp(&b.port()))
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! test_message_answer (
        ($name: ident, $filename: expr, $expected_message: expr) => (

          #[test]
          fn $name() {
            let data = include_str!($filename);
            let pretty_print = serde_json::to_string_pretty(&$expected_message)
                .expect("should have serialized");
            assert_eq!(
                &pretty_print,
                data,
                "\nserialized message:\n{}\n\nexpected message:\n{}",
                pretty_print,
                data
            );

            let message: Response = serde_json::from_str(data).unwrap();
            assert_eq!(
                message,
                $expected_message,
                "\ndeserialized message:\n{:#?}\n\nexpected message:\n{:#?}",
                message,
                $expected_message
            );
          }
        )
      );

    test_message_answer!(
        answer_workers_status,
        "../assets/answer_workers_status.json",
        Response {
            id: "ID_TEST".to_string(),
            version: 0,
            status: ResponseStatus::Ok,
            message: String::from(""),
            content: Some(ResponseContent::Workers(vec!(
                WorkerInfo {
                    id: 1,
                    pid: 5678,
                    run_state: RunState::Running,
                },
                WorkerInfo {
                    id: 0,
                    pid: 1234,
                    run_state: RunState::Stopping,
                },
            ))),
        }
    );

    test_message_answer!(
        answer_metrics,
        "../assets/answer_metrics.json",
        Response {
            id: "ID_TEST".to_string(),
            version: 0,
            status: ResponseStatus::Ok,
            message: String::from(""),
            content: Some(ResponseContent::Metrics(AggregatedMetricsData {
                main: [
                    (String::from("sozu.gauge"), FilteredData::Gauge(1)),
                    (String::from("sozu.count"), FilteredData::Count(-2)),
                    (String::from("sozu.time"), FilteredData::Time(1234)),
                ]
                .iter()
                .cloned()
                .collect(),
                workers: [(
                    String::from("0"),
                    QueryAnswer::Metrics(QueryAnswerMetrics::All(WorkerMetrics {
                        proxy: Some(
                            [
                                (String::from("sozu.gauge"), FilteredData::Gauge(1)),
                                (String::from("sozu.count"), FilteredData::Count(-2)),
                                (String::from("sozu.time"), FilteredData::Time(1234)),
                            ]
                            .iter()
                            .cloned()
                            .collect()
                        ),
                        clusters: Some(
                            [(
                                String::from("cluster_1"),
                                ClusterMetricsData {
                                    cluster: Some(
                                        [(
                                            String::from("request_time"),
                                            FilteredData::Percentiles(Percentiles {
                                                samples: 42,
                                                p_50: 1,
                                                p_90: 2,
                                                p_99: 10,
                                                p_99_9: 12,
                                                p_99_99: 20,
                                                p_99_999: 22,
                                                p_100: 30,
                                            })
                                        )]
                                        .iter()
                                        .cloned()
                                        .collect()
                                    ),
                                    backends: Some(
                                        [(
                                            String::from("cluster_1-0"),
                                            [
                                                (
                                                    String::from("bytes_in"),
                                                    FilteredData::Count(256)
                                                ),
                                                (
                                                    String::from("bytes_out"),
                                                    FilteredData::Count(128)
                                                ),
                                                (
                                                    String::from("percentiles"),
                                                    FilteredData::Percentiles(Percentiles {
                                                        samples: 42,
                                                        p_50: 1,
                                                        p_90: 2,
                                                        p_99: 10,
                                                        p_99_9: 12,
                                                        p_99_99: 20,
                                                        p_99_999: 22,
                                                        p_100: 30,
                                                    })
                                                )
                                            ]
                                            .iter()
                                            .cloned()
                                            .collect()
                                        )]
                                        .iter()
                                        .cloned()
                                        .collect()
                                    ),
                                }
                            )]
                            .iter()
                            .cloned()
                            .collect()
                        )
                    }))
                )]
                .iter()
                .cloned()
                .collect()
            }))
        }
    );
}
