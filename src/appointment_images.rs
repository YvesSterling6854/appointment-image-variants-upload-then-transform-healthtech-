use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct IntakeRequest {
    pub appointment_id: String,
    pub filename: String,
    pub image_base64: String,
}

#[derive(Debug, Serialize)]
pub struct IntakeResult {
    pub appointment_id: String,
    pub archive_key: String,
    pub variant: String,
    pub notification: String,
}

pub fn archive_key(appointment_id: &str, filename: &str) -> String {
    format!("appointments/{appointment_id}/{filename}")
}

pub fn patient_safe_notification(appointment_id: &str) -> String {
    format!("Appointment {appointment_id}: imaging is ready for clinical review.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_confirms_review_without_disclosing_the_image() {
        let notification = patient_safe_notification("apt-1042");
        assert_eq!(notification, "Appointment apt-1042: imaging is ready for clinical review.");
        assert!(!notification.contains("base64"));
        assert!(!notification.contains("scan.png"));
    }
}
