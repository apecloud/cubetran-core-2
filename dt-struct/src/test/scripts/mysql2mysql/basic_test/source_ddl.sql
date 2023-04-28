create database struct_it;

-- simple test
CREATE TABLE struct_it.table_test(id integer, text varchar(10) comment 'col comment test',primary key (id)) comment 'table comment test'; 

-- full column type
CREATE TABLE struct_it.full_column_type (id INT UNSIGNED AUTO_INCREMENT PRIMARY KEY,varchar_col VARCHAR(255) NOT NULL,char_col CHAR(10),text_col TEXT,tinyint_col TINYINT DEFAULT 0,smallint_col SMALLINT,mediumint_col MEDIUMINT,int_col INT,bigint_col BIGINT,float_col FLOAT(8, 2),double_col DOUBLE(16, 4),decimal_col DECIMAL(10, 2),date_col DATE,datetime_col DATETIME,timestamp_col TIMESTAMP DEFAULT CURRENT_TIMESTAMP,time_col TIME,year_col YEAR,binary_col BINARY(16),varbinary_col VARBINARY(255),blob_col BLOB,tinyblob_col TINYBLOB,mediumblob_col MEDIUMBLOB,longblob_col LONGBLOB,enum_col ENUM('value1', 'value2', 'value3'),set_col SET('option1', 'option2', 'option3'));

-- full index type
CREATE TABLE struct_it.full_index_type (id INT UNSIGNED AUTO_INCREMENT PRIMARY KEY,unique_col VARCHAR(255) NOT NULL,index_col VARCHAR(255),fulltext_col FULLTEXT,spatial_col POINT NOT NULL,simple_index_col VARCHAR(255),composite_index_col1 VARCHAR(255),composite_index_col2 VARCHAR(255),composite_index_col3 VARCHAR(255));

CREATE UNIQUE INDEX unique_index ON struct_it.full_index_type (unique_col);

CREATE INDEX index_index ON struct_it.full_index_type (index_col);

-- not support: CREATE FULLTEXT INDEX fulltext_index ON struct_it.full_index_type (fulltext_col);

-- not support: CREATE SPATIAL INDEX spatial_index ON struct_it.full_index_type (spatial_col);

CREATE INDEX simple_index ON struct_it.full_index_type (simple_index_col);

CREATE INDEX composite_index ON struct_it.full_index_type (composite_index_col1, composite_index_col2, composite_index_col3);