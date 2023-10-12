use actix_web::{get, post, web, HttpResponse, Responder};
use fplus_lib::core::{
    CompleteGovernanceReviewInfo, CompleteNewApplicationProposalInfo, CreateApplicationInfo,
    LDNApplication, RefillInfo
};

#[post("/application")]
pub async fn create(info: web::Json<CreateApplicationInfo>) -> impl Responder {
    match LDNApplication::new(info.into_inner()).await {
        Ok(app) => HttpResponse::Ok().body(format!(
            "Created new application for issue: {}",
            app.application_id.clone()
        )),
        Err(e) => {
            return HttpResponse::BadRequest().body(e.to_string());
        }
    }
}

#[post("/application/{id}/trigger")]
pub async fn trigger(
    id: web::Path<String>,
    info: web::Json<CompleteGovernanceReviewInfo>,
) -> impl Responder {
    let ldn_application = match LDNApplication::load(id.into_inner()).await {
        Ok(app) => app,
        Err(e) => {
            return HttpResponse::BadRequest().body(e.to_string());
        }
    };
    match ldn_application
        .complete_governance_review(info.into_inner())
        .await
    {
        Ok(app) => HttpResponse::Ok().body(serde_json::to_string_pretty(&app).unwrap()),
        Err(_) => {
            return HttpResponse::BadRequest().body("Application is not in the correct state");
        }
    }
}

#[post("/application/{id}/propose")]
pub async fn propose(
    id: web::Path<String>,
    info: web::Json<CompleteNewApplicationProposalInfo>,
) -> impl Responder {
    let ldn_application = match LDNApplication::load(id.into_inner()).await {
        Ok(app) => app,
        Err(e) => {
            return HttpResponse::BadRequest().body(e.to_string());
        }
    };
    match ldn_application
        .complete_new_application_proposal(info.into_inner())
        .await
    {
        Ok(app) => HttpResponse::Ok().body(serde_json::to_string_pretty(&app).unwrap()),
        Err(_) => {
            return HttpResponse::BadRequest().body("Application is not in the correct state");
        }
    }
}

#[post("/application/{id}/approve")]
pub async fn approve(
    id: web::Path<String>,
    info: web::Json<CompleteNewApplicationProposalInfo>,
) -> impl Responder {
    let ldn_application = match LDNApplication::load(id.into_inner()).await {
        Ok(app) => app,
        Err(e) => {
            return HttpResponse::BadRequest().body(e.to_string());
        }
    };
    match ldn_application
        .complete_new_application_approval(info.into_inner())
        .await
    {
        Ok(app) => HttpResponse::Ok().body(serde_json::to_string_pretty(&app).unwrap()),
        Err(_) => HttpResponse::BadRequest().body("Application is not in the correct state"),
    }
}

#[get("/application/active")]
pub async fn active() -> impl Responder {
    let apps = match LDNApplication::active().await {
        Ok(app) => app,
        Err(e) => {
           return  HttpResponse::BadRequest().body(e.to_string())
        }
    };
    HttpResponse::Ok().body(serde_json::to_string_pretty(&apps).unwrap())
}

#[get("/application/merged")]
pub async fn merged() -> actix_web::Result<impl Responder> {
    match LDNApplication::merged().await {
        Ok(apps) => Ok(HttpResponse::Ok().body(serde_json::to_string_pretty(&apps).unwrap())),
        Err(e) => {
            return Ok(HttpResponse::InternalServerError().body(e.to_string()));
        }
    }
}


#[post("/application/refill")]
pub async fn refill(data: web::Json<Vec<RefillInfo>>) -> actix_web::Result<impl Responder> {
    match LDNApplication::refill(data.0).await {
        Ok(applications) => Ok(HttpResponse::Ok().json(applications)),
        Err(e) => Ok(HttpResponse::BadRequest().body(e.to_string())),
    }
}

#[get("/health")]
pub async fn health() -> impl Responder {
    HttpResponse::Ok().body("OK")
}
