use futures::future;
use octocrab::models::{
    pulls::PullRequest,
    repos::{Content, ContentItems},
};
use reqwest::Response;
use serde::{Deserialize, Serialize};

use crate::{
    base64,
    error::LDNError,
    external_services::github::{
        CreateMergeRequestData, CreateRefillMergeRequestData, GithubWrapper,
    },
    parsers::ParsedIssue,
};

use self::application::file::{
    AllocationRequest, AllocationRequestType, AppState, ApplicationFile, Notary,
};

pub mod application;

#[derive(Deserialize)]
pub struct CreateApplicationInfo {
    pub issue_number: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct CompleteNewApplicationProposalInfo {
    signer: Notary,
    request_id: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ProposeApplicationInfo {
    uuid: String,
    client_address: String,
    notary_address: String,
    time_of_signature: String,
    message_cid: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ApproveApplicationInfo {
    uuid: String,
    client_address: String,
    notary_address: String,
    allocation_amount: String,
    time_of_signature: String,
    message_cid: String,
}

#[derive(Debug)]
pub struct LDNApplication {
    github: GithubWrapper<'static>,
    pub application_id: String,
    pub file_sha: String,
    pub file_name: String,
    pub branch_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompleteGovernanceReviewInfo {
    actor: String,
}

#[derive(Deserialize, Debug)]
pub struct RefillInfo {
    pub id: String,
    pub amount: String,
    pub amount_type: String,
}

impl LDNApplication {
    pub async fn single_active(pr_number: u64) -> Result<ApplicationFile, LDNError> {
        let gh: GithubWrapper = GithubWrapper::new();
        let (_, pull_request) = gh.get_pull_request_files(pr_number).await.unwrap();
        let pull_request = pull_request.get(0).unwrap();
        let pull_request: Response = reqwest::Client::new()
            .get(&pull_request.raw_url.to_string())
            .send()
            .await
            .map_err(|e| LDNError::Load(format!("Failed to get pull request files /// {}", e)))?;
        let pull_request = pull_request
            .text()
            .await
            .map_err(|e| LDNError::Load(format!("Failed to get pull request files /// {}", e)))?;
        if let Ok(app) = serde_json::from_str::<ApplicationFile>(&pull_request) {
            Ok(app)
        } else {
            Err(LDNError::Load(format!(
                "Pull Request {} Application file is corrupted",
                pr_number
            )))
        }
    }

    async fn load_pr_files(
        pr: PullRequest,
    ) -> Result<(String, String, ApplicationFile, PullRequest), LDNError> {
        let gh = GithubWrapper::new();
        let files = gh.get_pull_request_files(pr.number).await.unwrap();
        let response = reqwest::Client::new()
            .get(files.1.get(0).unwrap().raw_url.to_string())
            .send()
            .await;
        let response = response.unwrap();
        let response = response.text().await;
        let response = response.unwrap();
        let app = serde_json::from_str::<ApplicationFile>(&response).unwrap();
        Ok((
            files.1.get(0).unwrap().sha.clone(),
            files.1.get(0).unwrap().filename.clone(),
            app,
            pr.clone(),
        ))
    }

    pub async fn load(application_id: String) -> Result<Self, LDNError> {
        let gh: GithubWrapper = GithubWrapper::new();
        let pull_requests = gh.list_pull_requests().await.unwrap();
        let pull_requests = future::try_join_all(
            pull_requests
                .into_iter()
                .map(|pr: PullRequest| (LDNApplication::load_pr_files(pr)))
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
        for r in pull_requests {
            if String::from(r.2.id.clone()) == application_id.clone() {
                return Ok(Self {
                    github: gh,
                    application_id: r.2.id.clone(),
                    file_sha: r.0,
                    file_name: r.1,
                    branch_name: r.3.head.ref_field,
                });
            }
        }
        Err(LDNError::Load(format!("")))
    }

    pub async fn active(filter: Option<String>) -> Result<Vec<ApplicationFile>, LDNError> {
        let gh: GithubWrapper = GithubWrapper::new();
        let mut apps: Vec<ApplicationFile> = Vec::new();
        let pull_requests = gh.list_pull_requests().await.unwrap();
        let pull_requests = future::try_join_all(
            pull_requests
                .into_iter()
                .map(|pr: PullRequest| LDNApplication::load_pr_files(pr))
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
        for r in pull_requests {
            if filter.is_none() {
                apps.push(r.2)
            } else {
                if r.2.id == filter.clone().unwrap() {
                    apps.push(r.2)
                }
            }
        }
        Ok(apps)
    }

    /// Create New Application
    pub async fn new_from_issue(info: CreateApplicationInfo) -> Result<Self, LDNError> {
        let issue_number = info.issue_number;
        let gh: GithubWrapper = GithubWrapper::new();
        let (parsed_ldn, _) = LDNApplication::parse_application_issue(issue_number.clone()).await?;
        let application_id = parsed_ldn.id.clone();
        let app_path = LDNPullRequest::application_path(&application_id);
        let app_branch_name = LDNPullRequest::application_branch_name(&application_id);

        match gh.get_file(&app_path, &app_branch_name).await {
            Err(_) => {
                let file_sha = LDNPullRequest::create_empty_pr(
                    application_id.clone(),
                    parsed_ldn.client.name.clone(),
                    LDNPullRequest::application_branch_name(&issue_number),
                    None,
                )
                .await?;
                let application_file = ApplicationFile::new(
                    issue_number,
                    "MULTISIG ADDRESS".to_string(),
                    parsed_ldn.version,
                    parsed_ldn.id,
                    parsed_ldn.client.clone(),
                    parsed_ldn.project,
                    parsed_ldn.datacap,
                )
                .await;
                let file_content = match serde_json::to_string_pretty(&application_file) {
                    Ok(f) => f,
                    Err(e) => {
                        return Err(LDNError::New(format!(
                            "Application issue file is corrupted /// {}",
                            e
                        )))
                    }
                };
                let pr_handler =
                    LDNPullRequest::load(&application_id, &parsed_ldn.client.name.clone());
                pr_handler
                    .add_commit(
                        LDNPullRequest::application_move_to_governance_review(),
                        file_content,
                        file_sha.clone(),
                    )
                    .await;
                Ok(LDNApplication {
                    github: gh,
                    application_id,
                    file_sha,
                    file_name: pr_handler.path,
                    branch_name: pr_handler.branch_name,
                })
            }
            Ok(_) => {
                return Err(LDNError::New(format!(
                    "Application issue {} already exists",
                    application_id
                )))
            }
        }
    }

    /// Move application from Governance Review to Proposal
    pub async fn complete_governance_review(
        &self,
        info: CompleteGovernanceReviewInfo,
    ) -> Result<ApplicationFile, LDNError> {
        match self.app_state().await {
            Ok(s) => match s {
                AppState::GovernanceReview => {
                    let app_file: ApplicationFile = self.file().await?;
                    let uuid = uuidv4::uuid::v4();
                    let request = AllocationRequest::new(
                        info.actor.clone(),
                        uuid,
                        AllocationRequestType::First,
                        app_file.datacap.total_requested_amount.clone(),
                    );
                    let app_file = app_file.complete_governance_review(info.actor.clone(), request);
                    let file_content = serde_json::to_string_pretty(&app_file).unwrap();
                    match LDNPullRequest::add_commit_to(
                        self.file_name.clone(),
                        self.branch_name.clone(),
                        LDNPullRequest::application_move_to_proposal_commit(&info.actor),
                        file_content,
                        self.file_sha.clone(),
                    )
                    .await
                    {
                        Some(()) => Ok(app_file),
                        None => {
                            return Err(LDNError::New(format!(
                                "Application issue {} cannot be triggered(1)",
                                self.application_id
                            )))
                        }
                    }
                }
                _ => Err(LDNError::New(format!(
                    "Application issue {} cannot be triggered(2)",
                    self.application_id
                ))),
            },
            Err(e) => Err(LDNError::New(format!(
                "Application issue {} cannot be triggered {}(3)",
                self.application_id, e
            ))),
        }
    }

    /// Move application from Proposal to Approved
    pub async fn complete_new_application_proposal(
        &self,
        info: CompleteNewApplicationProposalInfo,
    ) -> Result<ApplicationFile, LDNError> {
        let CompleteNewApplicationProposalInfo { signer, request_id } = info;
        match self.app_state().await {
            Ok(s) => match s {
                AppState::ReadyToSign => {
                    let app_file: ApplicationFile = self.file().await?;
                    if !app_file.allocation.is_active(request_id.clone()) {
                        return Err(LDNError::Load(format!(
                            "Request {} is not active",
                            request_id
                        )));
                    }
                    let app_lifecycle = app_file.lifecycle.finish_proposal();
                    let app_file = app_file.add_signer_to_allocation(
                        signer.clone(),
                        request_id,
                        app_lifecycle,
                    );
                    let file_content = serde_json::to_string_pretty(&app_file).unwrap();
                    match LDNPullRequest::add_commit_to(
                        self.file_name.clone(),
                        self.branch_name.clone(),
                        LDNPullRequest::application_move_to_approval_commit(
                            &signer.signing_address,
                        ),
                        file_content,
                        self.file_sha.clone(),
                    )
                    .await
                    {
                        Some(()) => Ok(app_file),
                        None => {
                            return Err(LDNError::New(format!(
                                "Application issue {} cannot be proposed(1)",
                                self.application_id
                            )))
                        }
                    }
                }
                _ => Err(LDNError::New(format!(
                    "Application issue {} cannot be proposed(2)",
                    self.application_id
                ))),
            },
            Err(e) => Err(LDNError::New(format!(
                "Application issue {} cannot be proposed {}(3)",
                self.application_id, e
            ))),
        }
    }

    pub async fn complete_new_application_approval(
        &self,
        info: CompleteNewApplicationProposalInfo,
    ) -> Result<ApplicationFile, LDNError> {
        let CompleteNewApplicationProposalInfo { signer, request_id } = info;
        match self.app_state().await {
            Ok(s) => match s {
                AppState::StartSignDatacap => {
                    let app_file: ApplicationFile = self.file().await?;
                    let app_lifecycle = app_file.lifecycle.finish_approval();
                    let app_file = app_file.add_signer_to_allocation_and_complete(
                        signer.clone(),
                        request_id,
                        app_lifecycle,
                    );
                    let file_content = serde_json::to_string_pretty(&app_file).unwrap();
                    match LDNPullRequest::add_commit_to(
                        self.file_name.clone(),
                        self.branch_name.clone(),
                        LDNPullRequest::application_move_to_confirmed_commit(
                            &signer.signing_address,
                        ),
                        file_content,
                        self.file_sha.clone(),
                    )
                    .await
                    {
                        Some(()) => Ok(app_file),
                        None => {
                            return Err(LDNError::New(format!(
                                "Application issue {} cannot be proposed(1)",
                                self.application_id
                            )))
                        }
                    }
                }
                _ => Err(LDNError::New(format!(
                    "Application issue {} cannot be proposed(2)",
                    self.application_id
                ))),
            },
            Err(e) => Err(LDNError::New(format!(
                "Application issue {} cannot be proposed {}(3)",
                self.application_id, e
            ))),
        }
    }

    async fn parse_application_issue(
        issue_number: String,
    ) -> Result<(ParsedIssue, String), LDNError> {
        let gh: GithubWrapper = GithubWrapper::new();
        let issue = match gh.list_issue(issue_number.parse().unwrap()).await {
            Ok(issue) => issue,
            Err(e) => {
                return Err(LDNError::Load(format!(
                    "Application issue {} does not exist /// {}",
                    issue_number, e
                )))
            }
        };
        let issue_body = match issue.body {
            Some(body) => body,
            None => {
                return Err(LDNError::Load(format!(
                    "Application issue {} is empty",
                    issue_number
                )))
            }
        };
        Ok((ParsedIssue::from_issue_body(&issue_body), issue.user.login))
    }

    /// Return Application state
    async fn app_state(&self) -> Result<AppState, LDNError> {
        let f = self.file().await?;
        Ok(f.lifecycle.get_state())
    }

    /// Return Application state
    pub async fn total_dc_reached(application_id: String) -> Result<bool, LDNError> {
        let merged = Self::merged().await?;
        let app = match merged.iter().find(|(_, app)| app.id == application_id) {
            Some(app) => app,
            None => {
                return Err(LDNError::Load(format!(
                    "Application issue {} does not exist",
                    application_id
                )))
            }
        };
        match app.1.lifecycle.get_state() {
            AppState::Granted => {
                let app = app.1.reached_total_datacap();
                let gh: GithubWrapper<'_> = GithubWrapper::new();
                let ldn_app = LDNApplication::load(application_id.clone()).await?;
                let ContentItems { items } = gh.get_file(&ldn_app.file_name, "main").await.unwrap();

                LDNPullRequest::create_refill_pr(
                    app.id.clone(),
                    app.client.name.clone(),
                    items[0].sha.clone(),
                    serde_json::to_string_pretty(&app).unwrap(),
                )
                .await?;
                // let app_file: ApplicationFile = self.file().await?;
                // let file_content = serde_json::to_string_pretty(&app_file).unwrap();
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn content_items_to_app_file(file: ContentItems) -> Result<ApplicationFile, LDNError> {
        let f = match &file.items[0].content {
            Some(f) => f,
            None => return Err(LDNError::Load(format!("Application file is corrupted",))),
        };
        match base64::decode(&f.replace("\n", "")) {
            Some(f) => {
                return Ok(ApplicationFile::from(f));
            }
            None => {
                return Err(LDNError::Load(format!(
                    "Application issue file is corrupted",
                )))
            }
        }
    }

    async fn file(&self) -> Result<ApplicationFile, LDNError> {
        let app_path = LDNPullRequest::application_path(&self.application_id);
        let app_branch_name = LDNPullRequest::application_branch_name(&self.application_id);
        match self.github.get_file(&app_path, &app_branch_name).await {
            Ok(file) => Ok(LDNApplication::content_items_to_app_file(file)?),
            Err(e) => {
                return Err(LDNError::Load(format!(
                    "Application issue {} file does not exist /// {}",
                    self.application_id, e
                )))
            }
        }
    }

    async fn single_merged(application_id: String) -> Result<(Content, ApplicationFile), LDNError> {
        let merged = LDNApplication::merged().await?;
        let app = match merged.into_iter().find(|(_, app)| app.id == application_id) {
            Some(app) => Ok(app),
            None => Err(LDNError::Load(format!(
                "Application issue {} does not exist",
                application_id
            ))),
        };
        app
    }

    pub async fn think_merged(item: Content) -> Result<(Content, ApplicationFile), LDNError> {
        let file = reqwest::Client::new()
            .get(&item.download_url.clone().unwrap())
            .send()
            .await
            .map_err(|e| {
                LDNError::Load(format!(
                    "Failed to fetch application files from their URLs. Reason: {}",
                    e
                ))
            })?;
        let file = file.text().await.map_err(|e| {
            LDNError::Load(format!(
                "Failed to fetch application files from their URLs. Reason: {}",
                e
            ))
        })?;

        let app = match serde_json::from_str::<ApplicationFile>(&file) {
            Ok(app) => {
                if app.lifecycle.is_active {
                    app
                } else {
                    return Err(LDNError::Load(format!(
                        "Failed to fetch application files from their URLs",
                    )));
                }
            }
            Err(_) => {
                return Err(LDNError::Load(format!(
                    "Failed to fetch application files from their URLs",
                )));
            }
        };
        Ok((item, app))
    }

    pub async fn merged() -> Result<Vec<(Content, ApplicationFile)>, LDNError> {
        let gh = GithubWrapper::new();
        let all_files = gh.get_all_files().await.map_err(|e| {
            LDNError::Load(format!(
                "Failed to retrieve all files from GitHub. Reason: {}",
                e
            ))
        })?;
        let all_files = future::try_join_all(
            all_files
                .items
                .into_iter()
                .filter(|item: &Content| item.download_url.is_some())
                .map(|fd| LDNApplication::think_merged(fd))
                .collect::<Vec<_>>(),
        )
        .await
        .map_err(|e| {
            LDNError::Load(format!(
                "Failed to fetch application files from their URLs. Reason: {}",
                e
            ))
        })?;

        let mut apps: Vec<(Content, ApplicationFile)> = vec![];
        let active: Vec<ApplicationFile> = Self::active(None).await?;
        for app in all_files {
            if active.iter().find(|a| a.id == app.1.id).is_none() && app.1.lifecycle.is_active {
                apps.push(app);
            }
        }
        Ok(apps)
    }

    pub async fn refill(refill_info: RefillInfo) -> Result<bool, LDNError> {
        let apps = LDNApplication::merged().await?;
        if let Some((content, mut app)) = apps.into_iter().find(|(_, app)| app.id == refill_info.id)
        {
            let uuid = uuidv4::uuid::v4();
            let new_request = AllocationRequest::new(
                "SSA Bot".to_string(),
                uuid.clone(),
                AllocationRequestType::Refill(0),
                format!("{}{}", refill_info.amount, refill_info.amount_type),
            );
            let app_file = app.start_refill_request(new_request);
            LDNPullRequest::create_refill_pr(
                app.id.clone(),
                app.client.name.clone(),
                content.sha.clone(),
                serde_json::to_string_pretty(&app_file).unwrap(),
            )
            .await?;
            return Ok(true);
        }
        Err(LDNError::Load("Failed to get application file".to_string()))
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LDNPullRequest {
    pub branch_name: String,
    pub title: String,
    pub body: String,
    pub path: String,
}

impl LDNPullRequest {
    async fn create_empty_pr(
        application_id: String,
        owner_name: String,
        app_branch_name: String,
        base_hash: Option<String>,
    ) -> Result<String, LDNError> {
        let initial_commit = Self::application_initial_commit(&owner_name, &application_id);
        let gh: GithubWrapper = GithubWrapper::new();
        let create_ref_request =
            match gh.build_create_ref_request(app_branch_name.clone(), base_hash) {
                Ok(req) => req,
                Err(e) => {
                    return Err(LDNError::New(format!(
                        "Application issue cannot create branch request object /// {}",
                        e
                    )))
                }
            };

        let (_pr, file_sha) = match gh
            .create_merge_request(CreateMergeRequestData {
                application_id: application_id.clone(),
                owner_name,
                ref_request: create_ref_request,
                file_content: "{}".to_string(),
                commit: initial_commit,
            })
            .await
        {
            Ok((pr, file_sha)) => (pr, file_sha),
            Err(e) => {
                return Err(LDNError::New(format!(
                    "Application issue {} cannot create branch /// {}",
                    application_id, e
                )));
            }
        };
        Ok(file_sha)
    }

    async fn create_refill_pr(
        application_id: String,
        owner_name: String,
        file_sha: String,
        file_content: String,
    ) -> Result<u64, LDNError> {
        let initial_commit = Self::application_initial_commit(&owner_name, &application_id);
        let gh: GithubWrapper = GithubWrapper::new();
        let pr = match gh
            .create_refill_merge_request(CreateRefillMergeRequestData {
                application_id: application_id.clone(),
                owner_name,
                file_content,
                commit: initial_commit,
                file_sha,
            })
            .await
        {
            Ok(pr) => pr,
            Err(e) => {
                return Err(LDNError::New(format!(
                    "Application issue {} cannot create branch /// {}",
                    application_id, e
                )));
            }
        };
        Ok(pr.number)
    }

    pub(super) async fn add_commit_to(
        path: String,
        branch_name: String,
        commit_message: String,
        new_content: String,
        file_sha: String,
    ) -> Option<()> {
        let gh: GithubWrapper = GithubWrapper::new();
        match gh
            .update_file_content(
                &path,
                &commit_message,
                &new_content,
                &branch_name,
                &file_sha,
            )
            .await
        {
            Ok(_) => Some(()),
            Err(_) => None,
        }
    }

    pub(super) async fn add_commit(
        &self,
        commit_message: String,
        new_content: String,
        file_sha: String,
    ) -> Option<()> {
        let gh: GithubWrapper = GithubWrapper::new();
        match gh
            .update_file_content(
                &self.path,
                &commit_message,
                &new_content,
                &self.branch_name,
                &file_sha,
            )
            .await
        {
            Ok(_) => Some(()),
            Err(_) => None,
        }
    }

    pub(super) fn load(application_id: &str, owner_name: &str) -> Self {
        LDNPullRequest {
            branch_name: LDNPullRequest::application_branch_name(application_id),
            title: LDNPullRequest::application_title(application_id, owner_name),
            body: LDNPullRequest::application_body(application_id),
            path: LDNPullRequest::application_path(application_id),
        }
    }

    pub(super) fn application_branch_name(application_id: &str) -> String {
        format!("Application/{}", application_id)
    }

    pub(super) fn application_title(application_id: &str, owner_name: &str) -> String {
        format!("Application_{}_{}", application_id, owner_name)
    }

    pub(super) fn application_body(application_id: &str) -> String {
        format!("resolves #{}", application_id)
    }

    pub(super) fn application_path(application_id: &str) -> String {
        format!("{}.json", application_id)
    }

    pub(super) fn application_initial_commit(owner_name: &str, application_id: &str) -> String {
        format!("Start Application: {}-{}", owner_name, application_id)
    }

    pub(super) fn application_move_to_governance_review() -> String {
        format!("Application is under review of governance team")
    }

    pub(super) fn application_move_to_proposal_commit(actor: &str) -> String {
        format!(
            "Governance Team User {} Moved Application to Proposal State from Governance Review State",
            actor
        )
    }

    pub(super) fn application_move_to_approval_commit(actor: &str) -> String {
        format!(
            "Notary User {} Moved Application to Approval State from Proposal State",
            actor
        )
    }

    pub(super) fn application_move_to_confirmed_commit(actor: &str) -> String {
        format!(
            "Notary User {} Moved Application to Confirmed State from Proposal Approval",
            actor
        )
    }
}

pub fn get_file_sha(content: &ContentItems) -> Option<String> {
    match content.items.get(0) {
        Some(item) => {
            let sha = item.sha.clone();
            Some(sha)
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    // use super::*;
    // use octocrab::models::issues::Issue;
    // use tokio::time::{sleep, Duration};

    // #[tokio::test]
    // async fn end_to_end() {
    //     // Test Creating an application
    //     let gh: GithubWrapper = GithubWrapper::new();

    //     // let branches = gh.list_branches().await.unwrap();
    //     let issue = gh.list_issue(63).await.unwrap();
    //     let test_issue: Issue = gh
    //         .create_issue("from test", &issue.body.unwrap())
    //         .await
    //         .unwrap();
    //     assert!(LDNApplication::new(CreateApplicationInfo {
    //         application_id: test_issue.number.to_string(),
    //     })
    //     .await
    //     .is_ok());

    //     let application_id = test_issue.number.to_string();

    //     // validate file was created
    //     assert!(gh
    //         .get_file(
    //             &LDNPullRequest::application_path(application_id.as_str()),
    //             &LDNPullRequest::application_branch_name(application_id.as_str())
    //         )
    //         .await
    //         .is_ok());

    //     // validate pull request was created
    //     assert!(gh
    //         .get_pull_request_by_head(&LDNPullRequest::application_branch_name(
    //             application_id.as_str()
    //         ))
    //         .await
    //         .is_ok());

    //     // Test Triggering an application
    //     let ldn_application_before_trigger =
    //         LDNApplication::load(application_id.clone()).await.unwrap();
    //     ldn_application_before_trigger
    //         .complete_governance_review(CompleteGovernanceReviewInfo {
    //             actor: "actor_address".to_string(),
    //         })
    //         .await
    //         .unwrap();
    //     let ldn_application_after_trigger =
    //         LDNApplication::load(application_id.clone()).await.unwrap();
    //     assert_eq!(
    //         ldn_application_after_trigger.app_state().await.unwrap(),
    //         AppState::Proposal
    //     );
    //     dbg!("waiting for 2 second");
    //     sleep(Duration::from_millis(1000)).await;

    //     // // Test Proposing an application
    //     let ldn_application_after_trigger_success =
    //         LDNApplication::load(application_id.clone()).await.unwrap();
    //     let active_request_id = ldn_application_after_trigger_success
    //         .file()
    //         .await
    //         .unwrap()
    //         .info
    //         .application_lifecycle
    //         .get_active_allocation_id()
    //         .unwrap();
    //     ldn_application_after_trigger_success
    //         .complete_new_application_proposal(CompleteNewApplicationProposalInfo {
    //             request_id: active_request_id.clone(),
    //             signer: ApplicationAllocationsSigner {
    //                 signing_address: "signing_address".to_string(),
    //                 time_of_signature: "time_of_signature".to_string(),
    //                 message_cid: "message_cid".to_string(),
    //                 username: "gh_username".to_string(),
    //             },
    //         })
    //         .await
    //         .unwrap();

    //     let ldn_application_after_proposal =
    //         LDNApplication::load(application_id.clone()).await.unwrap();
    //     assert_eq!(
    //         ldn_application_after_proposal.app_state().await.unwrap(),
    //         AppState::Approval
    //     );
    //     dbg!("waiting for 2 second");
    //     sleep(Duration::from_millis(1000)).await;

    //     // Test Approving an application
    //     let ldn_application_after_proposal_success =
    //         LDNApplication::load(application_id.clone()).await.unwrap();
    //     ldn_application_after_proposal_success
    //         .complete_new_application_approval(CompleteNewApplicationProposalInfo {
    //             request_id: active_request_id.clone(),
    //             signer: ApplicationAllocationsSigner {
    //                 signing_address: "signing_address".to_string(),
    //                 time_of_signature: "time_of_signature".to_string(),
    //                 message_cid: "message_cid".to_string(),
    //                 username: "gh_username".to_string(),
    //             },
    //         })
    //         .await
    //         .unwrap();
    //     let ldn_application_after_approval =
    //         LDNApplication::load(application_id.clone()).await.unwrap();
    //     assert_eq!(
    //         ldn_application_after_approval.app_state().await.unwrap(),
    //         AppState::Confirmed
    //     );
    //     dbg!("waiting for 2 second");
    //     sleep(Duration::from_millis(1000)).await;

    //     // // Cleanup
    //     assert!(gh.close_issue(test_issue.number).await.is_ok());
    //     assert!(gh
    //         .close_pull_request(
    //             gh.get_pull_request_by_head(&LDNPullRequest::application_branch_name(
    //                 &application_id.clone()
    //             ))
    //             .await
    //             .unwrap()[0]
    //                 .number,
    //         )
    //         .await
    //         .is_ok());
    //     let remove_branch_request = gh
    //         .build_remove_ref_request(LDNPullRequest::application_branch_name(
    //             &application_id.clone(),
    //         ))
    //         .unwrap();
    //     assert!(gh.remove_branch(remove_branch_request).await.is_ok());
    // }
}
