table! {
  secret(id) {
    id -> Int4,
    jwt_secret -> Varchar,
  }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    aircrafts (aircrafts_id) {
        aircrafts_id -> Int8,
        ceiling -> Int4,
        country_of_origin -> Nullable<Int4>,
        crew_num -> Nullable<Int4>,
        description -> Nullable<Varchar>,
        content_body -> Nullable<Varchar>,
        img_url -> Nullable<Varchar>,
        landing_distance -> Nullable<Int4>,
        max_range_miles -> Int4,
        max_speed_miles_per_hour -> Int4,
        name -> Varchar,
        powerplant_number -> Nullable<Int4>,
        takeoff_distance -> Nullable<Int4>,
        wingspan_inches -> Nullable<Int4>,
        year_in_service -> Int4,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    aircrafts_countries (aircrafts_id, countries_id) {
        aircrafts_id -> Int8,
        countries_id -> Int4,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    aircrafts_images (aircrafts_id, images_id) {
        aircrafts_id -> Int8,
        images_id -> Int8,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    aircrafts_manufacturers (aircrafts_id, manufacturers_id) {
        aircrafts_id -> Int8,
        manufacturers_id -> Int8,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    aircrafts_powerplants (aircrafts_id, powerplants_id) {
        aircrafts_id -> Int8,
        powerplants_id -> Int8,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    countries (countries_id) {
        countries_id -> Int4,
        content_body -> Varchar,
        country_code -> Bpchar,
        name -> Bpchar,
        description -> Nullable<Varchar>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    ezy_course_c6 (course_id) {
        course_id -> Int4,
        tutor_id -> Int4,
        course_name -> Varchar,
        course_description -> Nullable<Varchar>,
        course_format -> Nullable<Varchar>,
        course_structure -> Nullable<Varchar>,
        course_duration -> Nullable<Varchar>,
        course_price -> Nullable<Int4>,
        course_language -> Nullable<Varchar>,
        course_level -> Nullable<Varchar>,
        posted_time -> Nullable<Timestamp>,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    ezy_tutor_c6 (tutor_id) {
        tutor_id -> Int4,
        tutor_name -> Varchar,
        tutor_pic_url -> Varchar,
        tutor_profile -> Varchar,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    images (images_id) {
        images_id -> Int8,
        name -> Varchar,
        ext -> Varchar,
        alt -> Varchar,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    manufacturers (manufacturers_id) {
        manufacturers_id -> Int8,
        content_body -> Nullable<Varchar>,
        country_of_origin_id -> Nullable<Int8>,
        description -> Nullable<Varchar>,
        name -> Varchar,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    manufacturers_countries (manufacturers_id, countries_id) {
        manufacturers_id -> Int8,
        countries_id -> Int8,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    use diesel::sql_types::*;
    use diesel_full_text_search::types::*;

    powerplants (powerplants_id) {
        powerplants_id -> Int8,
        content_body -> Nullable<Varchar>,
        description -> Varchar,
        hp -> Int8,
        title -> Nullable<Varchar>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

joinable!(aircrafts -> countries (country_of_origin));
joinable!(aircrafts_countries -> aircrafts (aircrafts_id));
joinable!(aircrafts_countries -> countries (countries_id));
joinable!(aircrafts_images -> aircrafts (aircrafts_id));
joinable!(aircrafts_images -> images (images_id));
joinable!(aircrafts_manufacturers -> aircrafts (aircrafts_id));
joinable!(aircrafts_manufacturers -> manufacturers (manufacturers_id));
joinable!(aircrafts_powerplants -> aircrafts (aircrafts_id));
joinable!(aircrafts_powerplants -> powerplants (powerplants_id));
joinable!(ezy_course_c6 -> ezy_tutor_c6 (tutor_id));
joinable!(manufacturers -> countries (country_of_origin_id));
joinable!(manufacturers_countries -> countries (countries_id));
joinable!(manufacturers_countries -> manufacturers (manufacturers_id));

allow_tables_to_appear_in_same_query!(
  aircrafts,
  aircrafts_countries,
  aircrafts_images,
  aircrafts_manufacturers,
  aircrafts_powerplants,
  countries,
  ezy_course_c6,
  ezy_tutor_c6,
  images,
  manufacturers,
  manufacturers_countries,
  powerplants,
);
